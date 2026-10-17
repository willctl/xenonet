//! core/components/rpc.js: a private, bounded local connection, JSON a
//! line. A request interrupted after it was sent is never sent again: an
//! action may already have taken effect. This is the connection's framing
//! and its rules; the socket, the pending requests and their timers stay
//! with whoever holds the socket.

use serde_json::Value;

/// The longest message either way, unless a connection says less.
pub const MAX_LINE: usize = 16 << 20;
/// How much may wait to be written before the connection counts as busy.
pub const MAX_WRITABLE: f64 = (16u64 << 20) as f64;

/// What a line that came in is.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    /// The answer to a request this side sent: its error (when there's one
    /// that's truthy), else its result.
    Reply { id: Value, result: Value, error: Option<String> },
    /// A request the other side sent.
    Request { id: f64, method: String, params: Value },
    /// An event this side takes.
    Event { name: String, value: Value },
    /// The other side broke the rules: the connection is to be dropped,
    /// and nothing after this read.
    Destroy,
}

pub struct Rpc {
    buffer: String,
    max_line: usize,
    events: Vec<String>,
    sequence: f64,
}

impl Rpc {
    /// `events`: the events this side takes; the transport's own
    /// (disconnect) and EventEmitter's special error never count.
    pub fn new(max_line: usize, events: Vec<String>) -> Self {
        Rpc { buffer: String::new(), max_line, events, sequence: 0.0 }
    }

    /// The next request's id.
    pub fn next_id(&mut self) -> f64 {
        self.sequence += 1.0;
        self.sequence
    }

    /// What a chunk that came in holds, line by line.
    pub fn data(&mut self, chunk: &str) -> Vec<Incoming> {
        self.buffer.push_str(chunk);
        let mut out = Vec::new();
        if self.buffer.len() > self.max_line {
            out.push(Incoming::Destroy);
            return out;
        }
        while let Some(end) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=end).collect();
            let line = &line[..line.len() - 1];
            let message = match xenonet_base::json::parse(line) {
                Ok(m @ (Value::Object(_) | Value::Array(_))) => m,
                _ => {
                    out.push(Incoming::Destroy);
                    break;
                }
            };
            let get = |k: &str| message.get(k).cloned();
            if xenonet_base::text::truthy(message.get("reply")) {
                let error = message.get("error").filter(|e| xenonet_base::text::truthy(Some(e))).map(xenonet_base::text::js_string);
                out.push(Incoming::Reply { id: get("reply").unwrap(), result: get("result").unwrap_or(Value::Null), error });
            } else if let (Some(Value::String(method)), Some(id)) = (message.get("method"), message.get("id").and_then(Value::as_f64).filter(|n| n.fract() == 0.0 && n.abs() <= 9007199254740991.0)) {
                out.push(Incoming::Request { id, method: method.clone(), params: get("params").unwrap_or(Value::Null) });
            } else if let Some(Value::String(name)) = message.get("event") {
                if self.events.iter().any(|e| e == name) {
                    out.push(Incoming::Event { name: name.clone(), value: get("value").unwrap_or(Value::Null) });
                }
            }
        }
        out
    }

    /// The line to write for `value`, or why it can't go: the connection
    /// is gone, or the line or what's waiting is too big.
    pub fn line(&self, value: &Value, gone: bool, writable: f64) -> Result<String, &'static str> {
        if gone {
            return Err("XenoNet disconnected; the action was not retried.");
        }
        let line = format!("{}\n", xenonet_base::json::stringify(value, 0));
        if line.len() > self.max_line || writable > MAX_WRITABLE {
            return Err("XenoNet's connection is too busy.");
        }
        Ok(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lines_in_and_out() {
        let mut r = Rpc::new(MAX_LINE, vec!["notification".into()]);
        let got = r.data("{\"reply\":1,\"result\":2}\n{\"event\":\"error\",\"value\":1}\n{\"event\":\"notification\",\"value\":3}\n{\"id\":4,\"method\":\"st");
        assert_eq!(got, [Incoming::Reply { id: json!(1), result: json!(2), error: None }, Incoming::Event { name: "notification".into(), value: json!(3) }]);
        assert_eq!(r.data("atus\"}\nnot json\n{\"x\":1}\n"), [Incoming::Request { id: 4.0, method: "status".into(), params: Value::Null }, Incoming::Destroy]);
        let mut small = Rpc::new(8, vec![]);
        assert_eq!(small.data("123456789"), [Incoming::Destroy]);
        assert!(r.line(&json!({ "id": 1 }), true, 0.0).is_err());
        assert_eq!(r.line(&json!({ "id": 1 }), false, 0.0).unwrap(), "{\"id\":1}\n");
    }
}
