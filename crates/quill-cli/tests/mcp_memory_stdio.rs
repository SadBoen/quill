//! `quill mcp memory` 的**端到端**测试：真的把那个子进程拉起来，手写 JSON-RPC 2.0
//! 在 stdio 上走一遍 `initialize` → `tools/list` → `tools/call`。
//!
//! **为什么手写协议、不用 rmcp 客户端。** 与 `quill-server/src/bin/mcp_stub.rs` 头注
//! 同一个道理：用同一份 rmcp 类型既当客户端又当服务端，一个协议层的错（方法名、字段、
//! 游标语义）会在两边同时犯，测不出来。这里只按 MCP 规范的字面来，服务端写错就会真的失败。
//! 而且这条路径顺带证明了一件产品上的事：**记忆的读写回路在进程外、过协议线也是通的**
//! （不是只有单元测试里那几个内存调用通）。
//!
//! 覆盖的回路：`remember_memory` 写进去 → `retrieve_memories` 读回来，
//! 并且工作目录经 `_meta` 的 `agent-working-dir` 传进去（本地记忆落在那里）。

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

const PROTOCOL: &str = "2025-06-18";
const WAIT: Duration = Duration::from_secs(20);

struct Server {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
}

impl Server {
    /// 拉起 `quill-cli mcp memory`。stdout 是协议通道、stderr 单独收（失败信息在那边）。
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_quill-cli"))
            .args(["mcp", "memory"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("拉起 quill-cli mcp memory");

        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(v) => {
                        let _ = tx.send(v);
                    }
                    // 协议行解不开 = 服务端往 stdout 打了非协议内容，这是缺陷。
                    Err(e) => panic!("stdout 上出现了不是 JSON 的行：{line:?}（{e}）"),
                }
            }
        });

        Server { child, stdin, rx }
    }

    fn send(&mut self, msg: &Value) {
        let line = serde_json::to_string(msg).expect("序列化");
        writeln!(self.stdin, "{line}").expect("写请求");
        self.stdin.flush().expect("flush");
    }

    /// 发一个带 id 的请求，等回同一个 id 的响应（跳过通知等其它帧）。
    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let v = self
                .rx
                .recv_timeout(WAIT)
                .unwrap_or_else(|_| panic!("等 {method} 的响应超时"));
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                return v;
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn handshake(s: &mut Server) {
    let init = s.request(
        1,
        "initialize",
        json!({
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": {"name": "quill-cli-e2e", "version": "0"}
        }),
    );
    assert_eq!(
        init["result"]["serverInfo"]["name"], "quill-memory",
        "服务器要自报名字，实际 {init}"
    );
    s.notify("notifications/initialized", json!({}));
}

fn tool_call(s: &mut Server, id: u64, name: &str, args: Value, working_dir: &str) -> Value {
    s.request(
        id,
        "tools/call",
        json!({
            "name": name,
            "arguments": args,
            "_meta": { "agent-working-dir": working_dir }
        }),
    )
}

/// 取 `tools/call` 结果里的文本（MCP 的 content[].text 拼接）。
fn result_text(v: &Value) -> String {
    let mut out = String::new();
    if let Some(items) = v["result"]["content"].as_array() {
        for it in items {
            if let Some(t) = it["text"].as_str() {
                out.push_str(t);
            }
        }
    }
    out
}

#[test]
fn memory_server_speaks_the_mcp_protocol_and_the_loop_works_over_the_wire() {
    let mut s = Server::start();
    handshake(&mut s);

    // 1) 工具面：四个，名字与上游一致。
    let list = s.request(2, "tools/list", json!({}));
    let names: Vec<String> = list["result"]["tools"]
        .as_array()
        .expect("tools 数组")
        .iter()
        .map(|t| t["name"].as_str().unwrap_or_default().to_string())
        .collect();
    for want in [
        "remember_memory",
        "retrieve_memories",
        "remove_memory_category",
        "remove_specific_memory",
    ] {
        assert!(
            names.iter().any(|n| n == want),
            "缺工具 {want}，实际 {names:?}"
        );
    }

    // 2) 写：本地分类，工作目录经 _meta 传进去。
    let work = std::env::temp_dir().join(format!("quill-mcp-memory-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    let work_s = work.to_string_lossy().to_string();

    let wrote = tool_call(
        &mut s,
        3,
        "remember_memory",
        json!({"category": "prefs", "data": "用户喜欢简洁的中文回复", "tags": ["style"], "is_global": false}),
        &work_s,
    );
    assert!(
        wrote.get("error").is_none(),
        "remember_memory 不该报错，实际 {wrote}"
    );
    assert!(
        result_text(&wrote).contains("prefs"),
        "回执要点名分类，实际 {wrote}"
    );

    // 真落盘：文件在工作目录的本地记忆目录里。
    let file = work.join(".quill").join("memory").join("prefs.txt");
    assert!(file.is_file(), "记忆应落在 {file:?}");
    let raw = std::fs::read_to_string(&file).unwrap();
    assert!(
        raw.contains("用户喜欢简洁的中文回复"),
        "落盘内容实际 {raw:?}"
    );

    // 3) 读：过协议线把刚写的读回来。
    let read = tool_call(
        &mut s,
        4,
        "retrieve_memories",
        json!({"category": "prefs", "is_global": false}),
        &work_s,
    );
    assert!(
        read.get("error").is_none(),
        "retrieve 不该报错，实际 {read}"
    );
    assert!(
        result_text(&read).contains("用户喜欢简洁的中文回复"),
        "读回来必须含刚写的内容，实际 {read}"
    );

    // 4) 非法分类名：服务端必须以**工具错误**回（不是崩、不是静默写出去）。
    let bad = tool_call(
        &mut s,
        5,
        "remember_memory",
        json!({"category": "../escape", "data": "x", "tags": [], "is_global": false}),
        &work_s,
    );
    assert!(bad.get("error").is_some(), "逃逸分类名必须报错，实际 {bad}");

    let _ = std::fs::remove_dir_all(&work);
}
