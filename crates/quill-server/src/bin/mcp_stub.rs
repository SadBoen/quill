//! 测试用的真·stdio MCP 服务器。**不是 mock** —— 它是一个真的子进程，
//! 通过 stdin/stdout 说 JSON-RPC 2.0，被 `mcp_client::probe` 真的拉起来、
//! 真的握手、真的 `tools/list`。
//!
//! **为什么手写而不是用 `rmcp::ServerHandler` 起一个 rmcp 服务器。**
//! 那样测的是「rmcp 客户端和 rmcp 服务端能不能对上」：两边共用同一份类型与
//! 版本协商，一个协议层的错（方法名写错、游标语义搞反、`initialize` 少一个字段）
//! 会在两边同时犯，测不出来。手写这一个只按 MCP 规范的字面来，客户端发错就会
//! 真的失败 —— 这才是这一层测试该有的样子。
//!
//! stdout **只能**是协议内容：任何一行多余的东西都会让客户端解析失败。
//! 要说话就写 stderr。
//!
//! 用法（集成测试通过 `CARGO_BIN_EXE_quill-mcp-stub` 拿到本文件的路径）：
//!
//! ```text
//! quill-mcp-stub                    正常：3 个工具，tools/list 分两页
//! quill-mcp-stub --one-page         不分页：3 个工具一次给全
//! quill-mcp-stub --no-tools         连上了但一个工具都没有
//! quill-mcp-stub --caps-off         initialize 报的能力里没有 tools
//! quill-mcp-stub --bad-initialize   initialize 直接回一个 JSON-RPC 错误
//! quill-mcp-stub --die-on-tools-list 握手成功后立刻退出
//! quill-mcp-stub --stutter          收到任何请求都不回话（测超时）
//! quill-mcp-stub --hold-open        stdin 关了也不退出（测子进程有没有被回收）
//!
//! 以下几支改的是 `tools/call`，用来测「真的调起来之后」的分支：
//! quill-mcp-stub --echo-args        把收到的 arguments 原样回进正文（验参数真的过了线）
//! quill-mcp-stub --call-error       回 isError=true，验失败不会被当成成功
//! quill-mcp-stub --call-structured  只回 structuredContent，不回任何文本
//! quill-mcp-stub --call-image       回一个图片块，验非文本不会被悄悄吞掉
//! quill-mcp-stub --call-die         收到 tools/call 就退出，验不假装成功
//! quill-mcp-stub --collide-tools    tools/list 报两个归一后同名的工具
//! ```

use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::{json, Value};

const PROTOCOL: &str = "2025-06-18";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);

    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            // 默认行为：stdin 一关就退出。`--hold-open` 故意不退出 ——
            // 这样「探测结束」就只可能来自传输层真的 kill 掉了我们，而不是
            // 「客户端关管道、我们顺势自己退了」。清理逻辑有没有做，只有这样测得出来。
            if has("--hold-open") {
                loop {
                    std::thread::sleep(Duration::from_secs(3600));
                }
            }
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            // 解析不了就当通知丢掉。真的 MCP 服务器不会因为一条坏行就自杀，
            // 客户端那边看到的是「没回话」，而不是「连接断了」。
            continue;
        };
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        // 通知没有 id，不能回响应 —— 回了就是多一条无关消息。
        let Some(id) = req.get("id").cloned() else {
            continue;
        };

        let result: Result<Value, Value> = match method {
            "initialize" => {
                if has("--bad-initialize") {
                    Err(json!({"code": -32603, "message": "stub 被要求握手失败"}))
                } else {
                    eprintln!("stub: 握手来自 {}", req["params"]["clientInfo"]["name"]);
                    Ok(json!({
                        "protocolVersion": PROTOCOL,
                        "capabilities": if has("--caps-off") { json!({}) } else { json!({"tools": {}}) },
                        "serverInfo": {"name": "quill-test-stub", "version": "1.0.0"}
                    }))
                }
            }
            "tools/list" => {
                if has("--die-on-tools-list") {
                    eprintln!("stub: 按要求在 tools/list 之前退出");
                    std::process::exit(3);
                }
                if has("--stutter") {
                    continue;
                }
                let cursor = req["params"]["cursor"].as_str().unwrap_or("");
                if has("--collide-tools") {
                    // 两个名字不同、但归一后挂载名相同（`a.b` 与 `a-b` 都压成 `a-b`）。
                    // 用来测「同名就跳过而不是顶掉」那条守卫在**真服务器**上走得通。
                    Ok(json!({"tools": vec![
                        tool("a.b", "带点的工具名"),
                        tool("a-b", "带连字符的工具名"),
                    ]}))
                } else if has("--one-page") {
                    Ok(json!({"tools": vec![
                        tool("read-note", "读一条笔记"),
                        tool("list-notes", "列出笔记"),
                        tool("echo-third", "第三页工具"),
                    ]}))
                } else if has("--no-tools") {
                    Ok(json!({"tools": []}))
                } else if cursor.is_empty() {
                    // 首页 2 个 + 游标。分页必须真的发生：客户端要是不翻第二页，
                    // 拼出来的工具数就是 2 而不是 3，而界面上看不出差。
                    Ok(json!({
                        "tools": vec![
                            tool("read-note", "读一条笔记"),
                            tool("list-notes", "列出笔记")
                        ],
                        "nextCursor": "page-2"
                    }))
                } else {
                    Ok(json!({"tools": vec![tool("echo-third", "第三页工具")]}))
                }
            }
            "tools/call" => {
                if has("--call-die") {
                    eprintln!("stub: 按要求在 tools/call 时退出");
                    std::process::exit(4);
                }
                let name = req["params"]["name"].as_str().unwrap_or("?");
                if has("--call-error") {
                    // 失败是**协议内的成功响应 + isError**，不是 JSON-RPC 错误。
                    // 这两者的区别正是下面那个测试要盯的。
                    Ok(json!({
                        "content": [{"type": "text", "text": format!("{name} 故意失败")}],
                        "isError": true
                    }))
                } else if has("--call-structured") {
                    Ok(json!({
                        "content": [],
                        "structuredContent": {"count": 7, "items": ["甲", "乙"]}
                    }))
                } else if has("--call-image") {
                    Ok(json!({"content": [
                        {"type": "image", "data": "AAAA", "mimeType": "image/png"},
                        {"type": "text", "text": "上面那张图"}
                    ]}))
                } else if has("--echo-args") {
                    // **把 arguments 原样回进正文。** 这样「参数真的过了线」这件事
                    // 就是被观察到的，而不是靠「调用没报错」间接推断 ——
                    // 一个把参数全丢掉的实现同样不会报错。
                    Ok(json!({"content": [{
                        "type": "text",
                        "text": format!(
                            "stub 执行了 {name}，收到参数 {}",
                            req["params"]["arguments"]
                        )
                    }]}))
                } else {
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": format!("stub 执行了 {name}")
                        }]
                    }))
                }
            }
            other => Err(json!({"code": -32601, "message": format!("stub 不认识 {other}")})),
        };

        let msg = match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
        };
        // 换行 + 立刻 flush：stdio 传输是按行读的，不 flush 客户端会一直等。
        if writeln!(out, "{msg}").is_err() || out.flush().is_err() {
            break;
        }
    }
}

fn tool(name: &str, description: &str) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": {"path": {"type": "string", "description": "笔记路径"}},
            "required": ["path"]
        }
    })
}
