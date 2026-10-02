//! Send one hand-built frame to the agent server from a running client.
//!
//! The headless net-check client can fire a raw frame before the session even
//! starts (`NETCHECK_PROBE`), which answers "does the server accept this
//! shape" but not "what does this do while I am standing in the world". This
//! is the same send, available mid-session over BRP, so a question about an
//! opcode costs a request instead of a login, a walk and a window.
//!
//! What it is **not**: it builds a frame and hands it to the existing outbound
//! queue, so every server-side check still applies, and it replaces no part of
//! the login. Two opcodes are refused outright — see [`refuses_opcode`].

use bevy::prelude::{In, Query, With};
use bevy::remote::error_codes::{INTERNAL_ERROR, INVALID_PARAMS};
use bevy::remote::{BrpError, BrpResult};
use bytes::Bytes;
use serde_json::{json, Value};

use crate::net::connection::SilkroadConnection;
use crate::net::frame::SilkroadFrame;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::plugin::carries_credentials;

/// The longest body this accepts. A frame's length field is 16 bits, and a
/// bigger body would be refused by the serialiser after the request looked
/// like it had worked.
const MAX_BODY_LEN: usize = u16::MAX as usize;

/// The two login opcodes carry an account password. The frame buffer already
/// withholds their bodies, and a tool that writes to the wire needs the same
/// line: nobody composes credentials through a developer method.
pub fn refuses_opcode(opcode: u16) -> bool {
    carries_credentials(opcode)
}

/// Reads `{opcode, body}`: the opcode as hex (`0x7021`, `7021`) or a decimal
/// number, the body as a hex string with optional spaces. An odd number of hex
/// digits is an error rather than a half-read byte.
pub fn parse_send_params(params: Option<&Value>) -> Result<(u16, Bytes), String> {
    let params = params.ok_or_else(|| "send needs {opcode, body}".to_string())?;
    let opcode = match params.get("opcode") {
        Some(Value::String(text)) => parse_opcode(text)?,
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| format!("opcode out of range: {number}"))?,
        Some(other) => return Err(format!("opcode must be a string or a number, got {other}")),
        None => return Err("send needs an opcode".to_string()),
    };
    if refuses_opcode(opcode) {
        return Err(format!(
            "opcode {opcode:#06x} carries account credentials and is refused here"
        ));
    }
    let body = match params.get("body") {
        None | Some(Value::Null) => Bytes::new(),
        Some(Value::String(hex)) => parse_hex_body(hex)?,
        Some(other) => return Err(format!("body must be a hex string, got {other}")),
    };
    if body.len() > MAX_BODY_LEN {
        return Err(format!(
            "body is {} bytes; a frame carries at most {MAX_BODY_LEN}",
            body.len()
        ));
    }
    Ok((opcode, body))
}

/// `0x7021`, `7021` as hex, or a plain decimal number.
fn parse_opcode(text: &str) -> Result<u16, String> {
    let trimmed = text.trim();
    let stripped = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"));
    match stripped {
        Some(hex) => u16::from_str_radix(hex, 16).map_err(|e| format!("bad opcode {text:?}: {e}")),
        None => u16::from_str_radix(trimmed, 16)
            .or_else(|_| trimmed.parse::<u16>())
            .map_err(|e| format!("bad opcode {text:?}: {e}")),
    }
}

/// Hex digits, optionally grouped by spaces so a body can be pasted in the
/// shape notes write it.
fn parse_hex_body(hex: &str) -> Result<Bytes, String> {
    let digits: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() % 2 != 0 {
        return Err(format!(
            "body has {} hex digits: a byte needs two",
            digits.len()
        ));
    }
    let mut bytes = Vec::with_capacity(digits.len() / 2);
    for index in (0..digits.len()).step_by(2) {
        let pair = &digits[index..index + 2];
        bytes.push(
            u8::from_str_radix(pair, 16).map_err(|e| format!("bad byte {pair:?} in body: {e}"))?,
        );
    }
    Ok(Bytes::from(bytes))
}

/// `openroad/packet_send`: queue one frame to the agent server.
pub fn brp_packet_send(
    In(params): In<Option<Value>>,
    connection: Query<&SilkroadConnection, With<AgentConnection>>,
) -> BrpResult {
    let (opcode, body) = parse_send_params(params.as_ref()).map_err(|message| BrpError {
        code: INVALID_PARAMS,
        message,
        data: None,
    })?;
    let Ok(connection) = connection.single() else {
        return Err(BrpError {
            code: INTERNAL_ERROR,
            message: "no agent connection: join a character first".to_string(),
            data: None,
        });
    };
    let len = body.len();
    // Counters and crc are what the outbound path fills in; a hand-built frame
    // uses the same zeroes the headless probe does.
    let frame = SilkroadFrame::Packet {
        count: 0,
        crc: 0,
        opcode,
        encrypted: 0,
        data: body,
    };
    connection.get_sender().send(frame).map_err(|e| BrpError {
        code: INTERNAL_ERROR,
        message: format!("the outbound queue refused the frame: {}", e.0),
        data: None,
    })?;
    // "Queued", not "sent": `send_packets` writes it on the next PostUpdate,
    // and the wire result shows up in `openroad/packet_tail`.
    Ok(json!({
        "queued": true,
        "opcode": format!("{opcode:#06x}"),
        "len": len,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gateway and agent logins, by the names the refusal is about.
    const GATEWAY_LOGIN: u16 = 0x6102;
    const AGENT_LOGIN: u16 = 0x6103;

    #[test]
    fn an_opcode_may_be_hex_or_decimal_and_a_body_is_optional() {
        let (opcode, body) =
            parse_send_params(Some(&json!({ "opcode": "0x7021" }))).expect("hex, no body");
        assert_eq!(opcode, 0x7021);
        assert!(body.is_empty());

        let (opcode, body) = parse_send_params(Some(&json!({ "opcode": "7021", "body": "01ff" })))
            .expect("bare hex with a body");
        assert_eq!(opcode, 0x7021);
        assert_eq!(&body[..], &[0x01, 0xff]);

        let (opcode, _) = parse_send_params(Some(&json!({ "opcode": 28705 }))).expect("decimal");
        assert_eq!(opcode, 0x7021);
    }

    /// A body is pasted from notes, which group bytes with spaces.
    #[test]
    fn a_body_may_be_grouped_by_spaces() {
        let (_, body) = parse_send_params(Some(&json!({ "opcode": "0x7021", "body": "01 02 ff" })))
            .expect("grouped hex");
        assert_eq!(&body[..], &[0x01, 0x02, 0xff]);
    }

    /// Half a byte is an error, not a guess: a dropped digit would send a
    /// different frame than the one that was asked for.
    #[test]
    fn an_odd_or_unreadable_body_is_refused() {
        let error = parse_send_params(Some(&json!({ "opcode": "0x7021", "body": "012" })))
            .expect_err("odd digit count");
        assert!(error.contains("two"), "{error}");
        assert!(parse_send_params(Some(&json!({ "opcode": "0x7021", "body": "zz" }))).is_err());
        assert!(parse_send_params(Some(&json!({ "opcode": "0x7021", "body": 12 }))).is_err());
    }

    #[test]
    fn a_request_without_an_opcode_is_refused() {
        assert!(parse_send_params(None).is_err());
        assert!(parse_send_params(Some(&json!({}))).is_err());
        assert!(parse_send_params(Some(&json!({ "opcode": "zzz" }))).is_err());
        assert!(parse_send_params(Some(&json!({ "opcode": 70000 }))).is_err());
        assert!(parse_send_params(Some(&json!({ "opcode": true }))).is_err());
    }

    /// The line this module exists to hold: the two login opcodes are refused
    /// here, locally and by name, so no developer method composes an account
    /// password. The frame buffer withholds their bodies; this one refuses to
    /// build them.
    #[test]
    fn the_two_credential_opcodes_are_refused_by_name() {
        for opcode in [GATEWAY_LOGIN, AGENT_LOGIN] {
            assert!(refuses_opcode(opcode), "{opcode:#06x}");
            let error = parse_send_params(Some(&json!({ "opcode": opcode })))
                .expect_err("a login opcode is refused");
            assert!(
                error.contains("credentials"),
                "the refusal has to say why: {error}"
            );
        }
        // A neighbouring opcode is not swept up by the refusal.
        assert!(!refuses_opcode(0x6104));
        assert!(parse_send_params(Some(&json!({ "opcode": "0x6104" }))).is_ok());
    }
}
