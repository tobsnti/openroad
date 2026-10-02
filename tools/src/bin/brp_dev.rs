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
use std::time::Duration;

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
