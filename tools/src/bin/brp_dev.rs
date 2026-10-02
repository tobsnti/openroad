//! A JSON-RPC client for the client's Bevy Remote Protocol server, for the
//! half of a session that *changes* something: take a screenshot, send keys or
//! a click, read or set a resource, query entities, end the process.
//!
//! `brp_perf` stays what it is — the measurement probe. Reading numbers and
//! driving state are different jobs: a measurement must not be able to move
//! the thing it measures, so they do not share a binary.
//!
//! Two properties are the point of this tool. It needs **no hotkey**: every
//! letter a dev binding would take is one the HUD wants (see the collision
//! table in `client/src/plugins/dev/mod.rs`). And it needs **no window of its
//! own**: input goes in through Bevy's event queues rather than the operating
//! system, so a probe works next to someone playing, and can be scripted in a
//! loop instead of performed in a session.
//!
//! The server is the `diagnostics` tier (`config.yaml`), HTTP on port 15702 by
//! default; `shutdown` exists so a probe can end its own client instead of
//! leaving a process behind that still holds an account.

use clap::{App, AppSettings, Arg, SubCommand};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `brp_extras/send_keys` refuses anything longer; failing here names the
/// limit instead of spending a round trip on it.
const MAX_KEY_DURATION_MS: u64 = 60_000;

struct Brp {
    url: String,
    agent: ureq::Agent,
}

impl Brp {
    fn new(host: &str, port: u16) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(2))
            .timeout(Duration::from_secs(20))
            .build();
        Self {
            url: format!("http://{host}:{port}/"),
            agent,
        }
    }

    fn call(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let mut request = json!({ "jsonrpc": "2.0", "id": 1, "method": method });
        if let Some(params) = params {
            request["params"] = params;
        }
        let response = self
            .agent
            .post(&self.url)
            .send_json(request)
            .map_err(|e| match e {
                ureq::Error::Transport(t) => format!(
                    "BRP not reachable at {url} ({t})\n\
                     \n\
                     Check, in order:\n\
                     - `diagnostics: true` in the config.yaml the client actually loaded\n\
                     \x20  (relative to the client's working directory, and restart-only).\n\
                     - the client is past loading.\n\
                     - you are on the same machine: the server binds 127.0.0.1.",
                    url = self.url
                ),
                other => format!("BRP request failed: {other}"),
            })?;
        let body: Value = response
            .into_json()
            .map_err(|e| format!("invalid BRP response: {e}"))?;
        if let Some(error) = body.get("error") {
            return Err(format!("BRP error from {method}: {error}"));
        }
        Ok(body.get("result").cloned().unwrap_or(Value::Null))
    }
}

// ---------------------------------------------------------------------------
// Request shaping. These are pure so the wire format is covered by tests
// instead of by a running client: a wrong field name is a silent no-op
// otherwise, because BRP answers a malformed request with an error object the
// caller has to read.
// ---------------------------------------------------------------------------

fn screenshot_request(path: &str) -> (&'static str, Value) {
    ("brp_extras/screenshot", json!({ "path": path }))
}

fn send_keys_request(keys: &[String], duration_ms: u64) -> Result<(&'static str, Value), String> {
    if keys.is_empty() {
        return Err("no keys given: e.g. brp_dev keys KeyI".into());
    }
    if duration_ms > MAX_KEY_DURATION_MS {
        return Err(format!(
            "--duration-ms {duration_ms} exceeds the {MAX_KEY_DURATION_MS} ms the server allows"
        ));
    }
    Ok((
        "brp_extras/send_keys",
        json!({ "keys": keys, "duration_ms": duration_ms }),
    ))
}

fn type_text_request(text: &str) -> (&'static str, Value) {
    ("brp_extras/type_text", json!({ "text": text }))
}

fn move_mouse_request(x: f32, y: f32) -> (&'static str, Value) {
    // glam serialises Vec2 as a two-element sequence, so the position is an
    // array and not {x, y}.
    ("brp_extras/move_mouse", json!({ "position": [x, y] }))
}

/// Bevy's `MouseButton` is an externally tagged enum, so the wire value is the
/// capitalised variant name.
fn mouse_button(raw: &str) -> Result<&'static str, String> {
    match raw.to_ascii_lowercase().as_str() {
        "left" => Ok("Left"),
        "right" => Ok("Right"),
        "middle" => Ok("Middle"),
        other => Err(format!(
            "unknown --button {other}: use left, right or middle"
        )),
    }
}

fn click_request(button: &str, double: bool) -> Result<(&'static str, Value), String> {
    let button = mouse_button(button)?;
    let method = if double {
        "brp_extras/double_click_mouse"
    } else {
        "brp_extras/click_mouse"
    };
    Ok((method, json!({ "button": button })))
}

fn resource_get_request(type_path: &str) -> (&'static str, Value) {
    ("world.get_resources", json!({ "resource": type_path }))
}

fn resource_set_request(type_path: &str, field: &str, raw_value: &str) -> (&'static str, Value) {
    // A value that parses as JSON is passed through as JSON (`false`, `3`,
    // `[1,2]`); anything else is a string, so `brp_dev res set … Foo` works.
    let value: Value =
        serde_json::from_str(raw_value).unwrap_or_else(|_| Value::String(raw_value.to_string()));
    (
        "world.mutate_resources",
        json!({ "resource": type_path, "path": field, "value": value }),
    )
}

/// Comma-separated type paths, with blanks dropped so `a,,b` and trailing
/// commas are not sent to the server as empty component names.
fn type_path_list(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn query_request(
    components: Option<&str>,
    option: Option<&str>,
    with: Option<&str>,
    without: Option<&str>,
) -> Result<(&'static str, Value), String> {
    let components = type_path_list(components);
    let option = type_path_list(option);
    let with = type_path_list(with);
    let without = type_path_list(without);
    if components.is_empty() && option.is_empty() && with.is_empty() {
        return Err("a query needs at least one of --components, --option or --with".into());
    }
    Ok((
        "world.query",
        json!({
            "data": { "components": components, "option": option, "has": [] },
            "filter": { "with": with, "without": without },
        }),
    ))
}

// ---------------------------------------------------------------------------
// `mark`: one folder per moment. Screenshot, the last frames, the UI rects and
// what the client is running, collected in one go so a report is evidence
// instead of a description.
// ---------------------------------------------------------------------------

/// Folder name for a mark taken at `unix_secs`, e.g. `mark-1790870000`.
/// Seconds, not milliseconds: two marks inside one second are rare, and a name
/// a person can read and type is worth more than that collision.
fn mark_dir_name(unix_secs: u64) -> String {
    format!("mark-{unix_secs}")
}

/// The screenshot path handed to the client must be **absolute**.
///
/// `brp_extras/screenshot` resolves a relative path in the *client's* working
/// directory, which is where the game was started, not where this command runs.
/// A relative `--out` would scatter PNGs next to the client while the mark
/// folder stays empty — and the folder would look like the screenshot failed.
fn absolute_screenshot_path(dir: &Path, file: &str) -> PathBuf {
    let joined = dir.join(file);
    if joined.is_absolute() {
        joined
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(joined)
    }
}

/// What this mark knows about the build it was taken against.
///
/// A folder of evidence that does not say which build produced it cannot be
/// read later: the same screenshot means different things on two commits. The
/// client is asked first (`openroad/build_info`); when it cannot say, the
/// field stays `null` **and carries the reason**, `mark` prints a warning, and
/// two weaker answers are recorded next to it — never *as* it:
///
/// * `stated` — what the operator passed on the command line.
/// * `tool_tree` — the commit of the tree this command ran in. That is **not
///   proof** of what the client is running; it is a hint for the common case
///   where the game was started from the same checkout, and it is labelled so
///   nobody reads it as the answer.
fn build_block(
    client: Option<&Value>,
    client_error: Option<&str>,
    stated: Option<&str>,
    tool_tree: Option<&str>,
    server: Option<&str>,
) -> Value {
    json!({
        "client": client.cloned().unwrap_or(Value::Null),
        "client_unknown_reason": match (client, client_error) {
            (Some(_), _) => Value::Null,
            (None, Some(reason)) => Value::String(reason.to_string()),
            (None, None) => Value::String("the client did not answer openroad/build_info".into()),
        },
        "stated": stated.map(|s| Value::String(s.to_string())).unwrap_or(Value::Null),
        "tool_tree": tool_tree.map(|s| Value::String(s.to_string())).unwrap_or(Value::Null),
        "tool_tree_note": "the checkout this command ran in, not proof of what the client runs",
        "server": server.map(|s| Value::String(s.to_string())).unwrap_or(Value::Null),
    })
}

/// `git rev-parse --short HEAD` plus a `-dirty` marker, or `None` when this is
/// not a git tree. Shelling out keeps the tool free of a git dependency.
fn tool_tree_commit() -> Option<String> {
    let head = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !head.status.success() {
        return None;
    }
    let mut commit = String::from_utf8(head.stdout).ok()?.trim().to_string();
    if commit.is_empty() {
        return None;
    }
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .is_some_and(|out| !out.stdout.is_empty());
    if dirty {
        commit.push_str("-dirty");
    }
    Some(commit)
}

/// True when the mark cannot name the client build — the one case where the
/// command says so out loud instead of writing a quiet `null`.
fn build_is_unknown(build: &Value) -> bool {
    build.get("client").map(Value::is_null).unwrap_or(true)
}

/// The index written as `mark.json`. Pure so its shape is tested without a
/// client: a mark that lists a file it did not write is worse than no mark.
#[allow(clippy::too_many_arguments)]
fn mark_index(
    unix_secs: u64,
    iso: &str,
    note: Option<&str>,
    build: Value,
    files: Vec<(&str, bool, Option<String>)>,
    frame_count: usize,
    node_count: usize,
) -> Value {
    let files: Vec<Value> = files
        .into_iter()
        .map(|(name, ok, error)| {
            json!({
                "file": name,
                "written": ok,
                // A step that failed keeps its line and says why, so a reader
                // sees "no screenshot because X", not an absent entry.
                "error": error.map(Value::String).unwrap_or(Value::Null),
            })
        })
        .collect();
    json!({
        "mark": mark_dir_name(unix_secs),
        "taken_at_unix": unix_secs,
        "taken_at": iso,
        "note": note.map(|n| Value::String(n.to_string())).unwrap_or(Value::Null),
        "build": build,
        "counts": { "frames": frame_count, "ui_nodes": node_count },
        "files": files,
    })
}

/// Seconds since the epoch, for the folder name and the index.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// RFC3339-ish stamp without pulling in a date crate: the client's own logs
/// use UTC, and a mark is read next to them.
fn iso_from_unix(unix_secs: u64) -> String {
    // Days since epoch -> civil date, the usual algorithm (Howard Hinnant's
    // `civil_from_days`), so the stamp needs no dependency.
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = unix_secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

fn write_json(dir: &Path, name: &str, value: &Value) -> Result<(), String> {
    let path = dir.join(name);
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Collect one moment. Every step is allowed to fail on its own: a mark with
/// three of four pieces is still evidence, as long as it says which piece is
/// missing and why.
#[allow(clippy::too_many_arguments)]
fn cmd_mark(
    brp: &Brp,
    out_root: &str,
    note: Option<&str>,
    packets: u64,
    stated_build: Option<&str>,
    server_build: Option<&str>,
) -> Result<(), String> {
    let unix_secs = unix_now();
    let dir = Path::new(out_root).join(mark_dir_name(unix_secs));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let mut files: Vec<(&str, bool, Option<String>)> = Vec::new();
    let mut frame_count = 0usize;
    let mut node_count = 0usize;

    // The build first: if the rest fails, the one thing that must be in the
    // folder is which client it was taken against.
    let (client_build, client_error) = match brp.call("openroad/build_info", None) {
        Ok(value) => (Some(value), None),
        Err(message) => (None, Some(message)),
    };
    let build = build_block(
        client_build.as_ref(),
        client_error.as_deref(),
        stated_build,
        tool_tree_commit().as_deref(),
        server_build,
    );

    match brp.call("openroad/packet_tail", Some(json!({ "limit": packets }))) {
        Ok(value) => {
            frame_count = value
                .get("frames")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let result = write_json(&dir, "packets.json", &value);
            files.push(("packets.json", result.is_ok(), result.err()));
        }
        Err(message) => files.push(("packets.json", false, Some(message))),
    }

    match brp.call("openroad/ui_rects", None) {
        Ok(value) => {
            node_count = value
                .get("nodes")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let result = write_json(&dir, "ui_rects.json", &value);
            files.push(("ui_rects.json", result.is_ok(), result.err()));
        }
        Err(message) => files.push(("ui_rects.json", false, Some(message))),
    }

    match brp.call("openroad/diagnostics", None) {
        Ok(value) => {
            let result = write_json(&dir, "diagnostics.json", &value);
            files.push(("diagnostics.json", result.is_ok(), result.err()));
        }
        Err(message) => files.push(("diagnostics.json", false, Some(message))),
    }

    let shot = absolute_screenshot_path(&dir, "shot.png");
    match brp.call(
        "brp_extras/screenshot",
        Some(json!({ "path": shot.display().to_string() })),
    ) {
        Ok(_) => files.push(("shot.png", true, None)),
        Err(message) => files.push(("shot.png", false, Some(message))),
    }

    let index = mark_index(
        unix_secs,
        &iso_from_unix(unix_secs),
        note,
        build,
        files,
        frame_count,
        node_count,
    );
    write_json(&dir, "mark.json", &index)?;

    println!("{}", dir.display());
    for entry in index["files"].as_array().into_iter().flatten() {
        let name = entry["file"].as_str().unwrap_or("?");
        match entry["error"].as_str() {
            None => println!("  {name}"),
            Some(error) => println!("  {name}  MISSING: {error}"),
        }
    }
    println!("  frames {frame_count} · ui nodes {node_count}");
    if build_is_unknown(&index["build"]) {
        // Loud on purpose: this is the gap that makes a folder of evidence
        // unreadable a day later.
        eprintln!(
            "warning: this mark does not name the client build ({}). \
             Evidence that cannot be tied to a build is hard to use later.",
            index["build"]["client_unknown_reason"]
                .as_str()
                .unwrap_or("unknown")
        );
    }
    Ok(())
}

fn print_result(result: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string())
    );
}

fn main() {
    let matches = App::new("brp_dev")
        .version("0.1.0")
        .about("Drive the client over its Bevy Remote Protocol server: screenshot, input, resources, queries, shutdown")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .arg(
            Arg::with_name("host")
                .long("host")
                .takes_value(true)
                .default_value("127.0.0.1")
                .global(true)
                .help("BRP server host"),
        )
        .arg(
            Arg::with_name("port")
                .long("port")
                .takes_value(true)
                .global(true)
                .help("BRP server port (default: $BRP_PORT, $BRP_EXTRAS_PORT, or 15702)"),
        )
        .subcommand(
            SubCommand::with_name("screenshot")
                .about("Save a PNG of the client window (no focus, no screencapture)")
                .arg(
                    Arg::with_name("out")
                        .long("out")
                        .takes_value(true)
                        .default_value("brp-dev.png")
                        .help("Output path; relative paths resolve in the client's working directory"),
                ),
        )
        .subcommand(
            SubCommand::with_name("keys")
                .about("Send KeyCode names, e.g. keys KeyI or keys ControlLeft KeyR")
                .arg(Arg::with_name("keys").multiple(true).required(true))
                .arg(
                    Arg::with_name("duration-ms")
                        .long("duration-ms")
                        .takes_value(true)
                        .default_value("100")
                        .help("How long the keys stay held"),
                ),
        )
        .subcommand(
            SubCommand::with_name("text")
                .about("Type a string into whatever has focus")
                .arg(Arg::with_name("text").required(true)),
        )
        .subcommand(
            SubCommand::with_name("move")
                .about("Move the cursor to an absolute window position")
                .arg(Arg::with_name("x").required(true))
                .arg(Arg::with_name("y").required(true)),
        )
        .subcommand(
            SubCommand::with_name("click")
                .about("Click at the cursor's current position (use `move` first)")
                .arg(
                    Arg::with_name("button")
                        .long("button")
                        .takes_value(true)
                        .default_value("left"),
                )
                .arg(
                    Arg::with_name("double")
                        .long("double")
                        .help("Double click instead of a single one"),
                ),
        )
        .subcommand(
            SubCommand::with_name("res")
                .about("List, read or set a reflected resource")
                .setting(AppSettings::SubcommandRequiredElseHelp)
                .subcommand(SubCommand::with_name("list").about("Every resource BRP can reach"))
                .subcommand(
                    SubCommand::with_name("get")
                        .about("Read one resource by full type path")
                        .arg(Arg::with_name("type-path").required(true)),
                )
                .subcommand(
                    SubCommand::with_name("set")
                        .about("Set one field of one resource")
                        .arg(Arg::with_name("type-path").required(true))
                        .arg(Arg::with_name("field").required(true))
                        .arg(Arg::with_name("value").required(true)),
                ),
        )
        .subcommand(
            SubCommand::with_name("query")
                .about("Query entities by component type paths (comma-separated)")
                .arg(Arg::with_name("components").long("components").takes_value(true))
                .arg(Arg::with_name("option").long("option").takes_value(true))
                .arg(Arg::with_name("with").long("with").takes_value(true))
                .arg(Arg::with_name("without").long("without").takes_value(true)),
        )
        .subcommand(
            SubCommand::with_name("mark")
                .about("Collect one moment into a folder: build, screenshot, last frames, UI rects")
                .arg(
                    Arg::with_name("out")
                        .long("out")
                        .takes_value(true)
                        .default_value("marks")
                        .help("Folder to create the mark in"),
                )
                .arg(
                    Arg::with_name("note")
                        .long("note")
                        .takes_value(true)
                        .help("What you saw, in your own words"),
                )
                .arg(
                    Arg::with_name("packets")
                        .long("packets")
                        .takes_value(true)
                        .default_value("50")
                        .help("How many recent frames to keep"),
                )
                .arg(
                    Arg::with_name("client-build")
                        .long("client-build")
                        .takes_value(true)
                        .help("Client commit, when you know it and the client cannot say"),
                )
                .arg(
                    Arg::with_name("server-build")
                        .long("server-build")
                        .takes_value(true)
                        .help("Server commit, when you know it: a mark that cannot name its build is hard to read later"),
                ),
        )
        .subcommand(
            SubCommand::with_name("shutdown")
                .about("End the client cleanly, so a probe leaves no process behind"),
        )
        .subcommand(
            SubCommand::with_name("raw")
                .about("Any BRP method by name, params as JSON")
                .arg(Arg::with_name("method").required(true))
                .arg(Arg::with_name("params").help("JSON object, omitted for none")),
        )
        .get_matches();

    let host = matches.value_of("host").unwrap_or("127.0.0.1").to_string();
    let port: u16 = matches
        .value_of("port")
        .map(str::to_string)
        .or_else(|| std::env::var("BRP_PORT").ok())
        .or_else(|| std::env::var("BRP_EXTRAS_PORT").ok())
        .map(|p| {
            p.parse()
                .unwrap_or_else(|_| exit_with(&format!("invalid port: {p}")))
        })
        .unwrap_or(15702);
    let brp = Brp::new(&host, port);

    let parse_f32 = |raw: &str| -> f32 {
        raw.parse()
            .unwrap_or_else(|_| exit_with(&format!("not a number: {raw}")))
    };

    // `mark` is not one request but several, so it runs here and returns.
    if let ("mark", Some(m)) = matches.subcommand() {
        let raw = m.value_of("packets").unwrap();
        let packets: u64 = raw
            .parse()
            .unwrap_or_else(|_| exit_with(&format!("invalid --packets: {raw}")));
        match cmd_mark(
            &brp,
            m.value_of("out").unwrap(),
            m.value_of("note"),
            packets,
            m.value_of("client-build"),
            m.value_of("server-build"),
        ) {
            Ok(()) => return,
            Err(message) => exit_with(&message),
        }
    }

    let request: Result<(&str, Option<Value>), String> = match matches.subcommand() {
        ("screenshot", Some(m)) => {
            let (method, params) = screenshot_request(m.value_of("out").unwrap());
            Ok((method, Some(params)))
        }
        ("keys", Some(m)) => {
            let keys: Vec<String> = m
                .values_of("keys")
                .map(|v| v.map(str::to_string).collect())
                .unwrap_or_default();
            let raw = m.value_of("duration-ms").unwrap();
            let duration_ms: u64 = raw
                .parse()
                .unwrap_or_else(|_| exit_with(&format!("invalid --duration-ms: {raw}")));
            send_keys_request(&keys, duration_ms).map(|(method, params)| (method, Some(params)))
        }
        ("text", Some(m)) => {
            let (method, params) = type_text_request(m.value_of("text").unwrap());
            Ok((method, Some(params)))
        }
        ("move", Some(m)) => {
            let (method, params) = move_mouse_request(
                parse_f32(m.value_of("x").unwrap()),
                parse_f32(m.value_of("y").unwrap()),
            );
            Ok((method, Some(params)))
        }
        ("click", Some(m)) => click_request(m.value_of("button").unwrap(), m.is_present("double"))
            .map(|(method, params)| (method, Some(params))),
        ("res", Some(m)) => match m.subcommand() {
            ("list", Some(_)) => Ok(("world.list_resources", None)),
            ("get", Some(m)) => {
                let (method, params) = resource_get_request(m.value_of("type-path").unwrap());
                Ok((method, Some(params)))
            }
            ("set", Some(m)) => {
                let (method, params) = resource_set_request(
                    m.value_of("type-path").unwrap(),
                    m.value_of("field").unwrap(),
                    m.value_of("value").unwrap(),
                );
                Ok((method, Some(params)))
            }
            _ => unreachable!("SubcommandRequiredElseHelp"),
        },
        ("query", Some(m)) => query_request(
            m.value_of("components"),
            m.value_of("option"),
            m.value_of("with"),
            m.value_of("without"),
        )
        .map(|(method, params)| (method, Some(params))),
        ("shutdown", Some(_)) => Ok(("brp_extras/shutdown", None)),
        ("raw", Some(m)) => {
            let method = m.value_of("method").unwrap();
            match m.value_of("params") {
                None => Ok((method, None)),
                Some(raw) => serde_json::from_str::<Value>(raw)
                    .map(|params| (method, Some(params)))
                    .map_err(|e| format!("params is not JSON: {e}")),
            }
        }
        _ => unreachable!("SubcommandRequiredElseHelp"),
    };

    match request.and_then(|(method, params)| brp.call(method, params)) {
        Ok(result) => print_result(&result),
        Err(message) => exit_with(&message),
    }
}

fn exit_with(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The server reads `path`; a different key would be accepted by this
    /// binary and refused by the client, which is the failure mode these
    /// tests exist for.
    #[test]
    fn a_screenshot_names_its_path() {
        let (method, params) = screenshot_request("/tmp/shot.png");
        assert_eq!(method, "brp_extras/screenshot");
        assert_eq!(params, json!({ "path": "/tmp/shot.png" }));
    }

    #[test]
    fn keys_carry_their_hold_duration() {
        let keys = vec!["ControlLeft".to_string(), "KeyR".to_string()];
        let (method, params) = send_keys_request(&keys, 250).expect("250 ms is allowed");
        assert_eq!(method, "brp_extras/send_keys");
        assert_eq!(
            params,
            json!({ "keys": ["ControlLeft", "KeyR"], "duration_ms": 250 })
        );
    }

    /// Both refusals happen here rather than after a round trip, and both name
    /// what to do instead.
    #[test]
    fn an_empty_or_overlong_key_press_is_refused_locally() {
        assert!(send_keys_request(&[], 100).is_err());
        let error = send_keys_request(&["KeyI".to_string()], MAX_KEY_DURATION_MS + 1)
            .expect_err("over the limit");
        assert!(error.contains("60000"), "{error}");
    }

    /// `move_mouse` takes a glam `Vec2`, which is a sequence on the wire. An
    /// object would deserialise to nothing and move nothing.
    #[test]
    fn a_cursor_move_sends_an_array_position() {
        let (method, params) = move_mouse_request(12.0, 34.5);
        assert_eq!(method, "brp_extras/move_mouse");
        assert_eq!(params, json!({ "position": [12.0, 34.5] }));
    }

    #[test]
    fn a_double_click_is_its_own_method_and_the_button_is_capitalised() {
        let (single, params) = click_request("right", false).expect("right is a button");
        assert_eq!(single, "brp_extras/click_mouse");
        assert_eq!(params, json!({ "button": "Right" }));
        let (double, _) = click_request("left", true).expect("left is a button");
        assert_eq!(double, "brp_extras/double_click_mouse");
        assert!(click_request("scroll", false).is_err());
    }

    /// A JSON value stays JSON, and everything else becomes a string, so both
    /// `false` and a plain word reach the server as the type it expects.
    #[test]
    fn setting_a_field_passes_json_through_and_quotes_the_rest() {
        let (method, params) = resource_set_request("some::Settings", "render_water", "false");
        assert_eq!(method, "world.mutate_resources");
        assert_eq!(params["value"], json!(false));
        assert_eq!(params["path"], json!("render_water"));
        assert_eq!(params["resource"], json!("some::Settings"));

        let (_, params) = resource_set_request("some::Settings", "name", "Wide");
        assert_eq!(params["value"], json!("Wide"));
    }

    /// Empty entries are dropped: an empty component name is a server-side
    /// error that reads like a missing type, not like a stray comma.
    #[test]
    fn a_type_path_list_drops_blanks() {
        assert_eq!(
            type_path_list(Some("a::B, c::D ,")),
            vec!["a::B".to_string(), "c::D".to_string()]
        );
        assert!(type_path_list(None).is_empty());
        assert!(type_path_list(Some(" , ")).is_empty());
    }

    /// Two marks must not be able to collide in a way a person cannot see, and
    /// the name has to be typable.
    #[test]
    fn a_mark_folder_is_named_after_its_second() {
        assert_eq!(mark_dir_name(0), "mark-0");
        assert_eq!(mark_dir_name(1_790_870_000), "mark-1790870000");
    }

    /// The trap this command would otherwise walk into: `brp_extras/screenshot`
    /// resolves a relative path in the CLIENT's working directory, so a
    /// relative mark folder would leave the PNG next to the game and the mark
    /// folder empty.
    #[test]
    fn a_screenshot_path_is_made_absolute_before_it_is_sent() {
        let absolute = absolute_screenshot_path(Path::new("/tmp/marks/mark-1"), "shot.png");
        assert_eq!(absolute, PathBuf::from("/tmp/marks/mark-1/shot.png"));
        let relative = absolute_screenshot_path(Path::new("marks/mark-1"), "shot.png");
        assert!(
            relative.is_absolute(),
            "a relative folder must still produce an absolute path: {}",
            relative.display()
        );
        assert!(relative.ends_with("marks/mark-1/shot.png"));
    }

    /// A folder of evidence that cannot name its build is the failure this
    /// field exists for, so the reason is carried instead of a bare `null` —
    /// and the two weaker answers sit beside it, never in its place.
    #[test]
    fn an_unknown_client_build_keeps_its_reason_and_its_weaker_answers() {
        let unknown = build_block(
            None,
            Some("connection refused"),
            None,
            Some("abc1234"),
            None,
        );
        assert!(build_is_unknown(&unknown));
        assert_eq!(unknown["client"], Value::Null);
        assert_eq!(
            unknown["client_unknown_reason"],
            json!("connection refused")
        );
        // The tree commit must NOT be promoted into `client`: it says what was
        // checked out here, not what the running client was built from.
        assert_eq!(unknown["tool_tree"], json!("abc1234"));
        assert!(unknown["tool_tree_note"]
            .as_str()
            .unwrap()
            .contains("not proof"));

        let stated = build_block(None, None, Some("deadbee"), None, None);
        assert!(
            build_is_unknown(&stated),
            "an operator's word is recorded, but it does not make the build known"
        );
        assert_eq!(stated["stated"], json!("deadbee"));

        let known = build_block(
            Some(&json!({ "commit": "0f1a037" })),
            None,
            None,
            Some("abc1234"),
            Some("srv-abc"),
        );
        assert!(!build_is_unknown(&known));
        assert_eq!(known["client"]["commit"], json!("0f1a037"));
        assert_eq!(known["client_unknown_reason"], Value::Null);
        assert_eq!(known["server"], json!("srv-abc"));
    }

    /// A step that failed keeps its line and says why: an absent entry would
    /// read as "not attempted", which is a different thing.
    #[test]
    fn the_index_lists_failed_steps_with_their_reason() {
        let index = mark_index(
            1_790_870_000,
            "2026-10-02T09:13:20Z",
            Some("the group window is empty"),
            build_block(
                Some(&json!({ "commit": "abc1234" })),
                None,
                None,
                None,
                None,
            ),
            vec![
                ("packets.json", true, None),
                ("shot.png", false, Some("BRP error: no window".to_string())),
            ],
            12,
            37,
        );
        assert_eq!(index["mark"], json!("mark-1790870000"));
        assert_eq!(index["note"], json!("the group window is empty"));
        assert_eq!(index["counts"]["frames"], json!(12));
        assert_eq!(index["counts"]["ui_nodes"], json!(37));
        let files = index["files"].as_array().expect("files is a list");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["written"], json!(true));
        assert_eq!(files[0]["error"], Value::Null);
        assert_eq!(files[1]["written"], json!(false));
        assert_eq!(files[1]["error"], json!("BRP error: no window"));
    }

    /// The stamp is read next to the client's own UTC logs, so it must be the
    /// same instant and in the same shape. Checked against known dates.
    #[test]
    fn the_timestamp_is_utc_and_matches_known_instants() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(1_000_000_000), "2001-09-09T01:46:40Z");
        // A leap day, where a wrong civil-date conversion slips by a day.
        assert_eq!(iso_from_unix(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(iso_from_unix(1_790_870_000), "2026-10-02T09:13:20Z");
    }

    #[test]
    fn a_query_needs_something_to_select_and_keeps_its_filter_halves_apart() {
        assert!(query_request(None, None, None, None).is_err());
        assert!(query_request(None, None, None, Some("a::B")).is_err());
        let (method, params) =
            query_request(Some("a::B"), None, Some("c::D"), Some("e::F")).expect("has components");
        assert_eq!(method, "world.query");
        assert_eq!(params["data"]["components"], json!(["a::B"]));
        assert_eq!(params["filter"]["with"], json!(["c::D"]));
        assert_eq!(params["filter"]["without"], json!(["e::F"]));
    }
}
