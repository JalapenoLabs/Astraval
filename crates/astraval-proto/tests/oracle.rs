// SPDX-License-Identifier: Apache-2.0

//! Live integration test against a real Perforce server (the "oracle").
//!
//! This proves the framing codec and connection work against genuine `p4d`, not
//! just against themselves. It is `#[ignore]`d so the normal `cargo test` run
//! stays green with no server present; run it explicitly when an oracle is up:
//!
//! ```text
//! # start the oracle (see protocol-lab/scripts/run-oracle.ps1), then:
//! cargo test -p astraval-proto --test oracle -- --ignored
//! ```
//!
//! Override the address with `ASTRAVAL_ORACLE_ADDR` (default `localhost:1666`).
//!
//! # What is and is not tested here
//!
//! This is issue #8: transport and framing only. The exchange mirrors the start
//! of a real session as seen in `protocol-lab/captures/02-rpc-framing.md`: the
//! client sends a `protocol` opener immediately followed by a command frame
//! (`user-info`), then reads the server's framed reply. We assert only that a
//! real `p4d` accepted our framing and answered with a well-formed frame we
//! decode. We do not interpret the handshake (issue #9) or dispatch the reply
//! (issue #10).
//!
//! A read timeout guards against the server blocking forever if a future
//! protocol change stops it from replying to this minimal opener.

use std::env;
use std::time::Duration;

use astraval_proto::handshake::{Protocol, negotiate};
use astraval_proto::{Connection, Message};

/// How long to wait for the oracle's reply before failing rather than hanging.
///
/// Generous for a localhost round trip; its only job is to turn a non-responsive
/// server into a clear test failure instead of an indefinite block.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// The oracle address, overridable for non-default test setups.
fn oracle_addr() -> String {
    env::var("ASTRAVAL_ORACLE_ADDR").unwrap_or_else(|_| "localhost:1666".to_string())
}

#[test]
#[ignore = "requires a live p4d oracle; run with --ignored"]
fn real_server_accepts_our_framing_and_replies() {
    let mut conn = Connection::connect(oracle_addr()).expect("connect to oracle");
    conn.transport()
        .set_read_timeout(Some(REPLY_TIMEOUT))
        .expect("set read timeout");

    // The `protocol` opener declares client capabilities; field names mirror the
    // captured handshake. A real `p4d` only replies once the command frame that
    // follows the opener arrives, so we send `user-info` right after, exactly as
    // `p4 info` does on the wire.
    let opener = Message::new()
        .with("func", "protocol")
        .with("client", "100")
        .with("api", "99999")
        .with("host", "astraval-test")
        .with("port", "tcp:localhost:1666");
    let command = Message::new()
        .with("func", "user-info")
        .with("prog", "astraval-test")
        .with("client", "astraval-test")
        .with("user", "astraval-test");

    conn.send(&opener).expect("send protocol opener");
    conn.send(&command).expect("send user-info command");

    let reply = conn.receive().expect("receive a framed reply");

    // We do not interpret the handshake here (that is a later issue); we only
    // require that a real server produced a valid, non-empty frame we decoded.
    assert!(
        !reply.fields().is_empty(),
        "oracle reply should carry at least one field, got {reply:?}"
    );
}

#[test]
#[ignore = "requires a live p4d oracle; run with --ignored"]
fn real_server_completes_protocol_negotiation() {
    // The full issue #9 handshake against a real `p4d`: send the `protocol`
    // opener, send a command, and parse the server's `protocol` reply into the
    // negotiated levels. Completing this is strong evidence the handshake is
    // wire-accurate, not merely self-consistent.
    let mut conn = Connection::connect(oracle_addr()).expect("connect to oracle");
    conn.transport()
        .set_read_timeout(Some(REPLY_TIMEOUT))
        .expect("set read timeout");

    let opener = Protocol::new("astraval-test", format!("tcp:{}", oracle_addr()));
    // A light, side-effect-free command; the server's `protocol` reply rides on
    // the front of the response to it.
    let command = Message::new()
        .with("func", "user-info")
        .with("prog", "astraval-test")
        .with("client", "astraval-test")
        .with("user", "astraval-test");

    let negotiated = negotiate(&mut conn, &opener, &command).expect("complete the handshake");

    // We do not assert exact levels (they vary by server build); we require that
    // a real server settled on a sane, positive server API level with us.
    assert!(
        negotiated.server_api_level() > 0,
        "oracle should negotiate a positive server API level, got {negotiated:?}"
    );
}
