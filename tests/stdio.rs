use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

#[test]
fn real_stdio_protocol_is_clean_and_discovery_is_offline() {
    let temp = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ametller-mcp"))
        .args(["--credential-store", "file", "--state-dir"])
        .arg(temp.path().join("state"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            tx.send(line.unwrap()).unwrap();
        }
    });
    let send = |input: &mut std::process::ChildStdin, value: Value| {
        writeln!(input, "{value}").unwrap();
        input.flush().unwrap();
    };
    let receive = || -> Value {
        serde_json::from_str(
            &rx.recv_timeout(Duration::from_secs(10))
                .expect("MCP response timed out"),
        )
        .expect("stdout must contain only JSON-RPC")
    };
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
    );
    let init = receive();
    assert_eq!(init["id"], 1);
    assert!(init["result"]["capabilities"]["tools"].is_object());
    send(
        &mut input,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    );
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    );
    let list = receive();
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 5);
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"authentication_status","arguments":{}}}),
    );
    let status = receive();
    assert_eq!(
        status["result"]["structuredContent"]["authenticated"],
        false
    );
    assert!(
        !temp.path().join("state").exists(),
        "discovery and offline status must not create state"
    );
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_cart","arguments":{}}}),
    );
    let cart = receive();
    assert_eq!(cart["result"]["isError"], true);
    assert!(cart.to_string().contains("Sign in privately"));
    drop(input);
    assert!(child.wait().unwrap().success());
    reader.join().unwrap();
    assert!(rx.try_recv().is_err());
}

#[test]
fn credentials_cannot_be_piped_or_passed_as_arguments() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ametller-mcp"))
        .args(["--credential-store", "file", "--state-dir"])
        .arg(temp.path().join("state"))
        .arg("login")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("interactive terminal"));
    assert!(!temp.path().join("state").exists());
}
