//! Mock FreeSWITCH ESL server for integration testing
#![allow(dead_code)]

use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

pub struct MockEslServer {
    listener: TcpListener,
    password: String,
}

pub struct MockClient {
    stream: TcpStream,
}

impl MockEslServer {
    pub async fn start(password: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        Self {
            listener,
            password: password.to_string(),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.listener
            .local_addr()
            .unwrap()
    }

    pub fn port(&self) -> u16 {
        self.addr()
            .port()
    }

    /// Accept a connection and perform the auth handshake
    pub async fn accept(&self) -> MockClient {
        let (stream, _addr) = self
            .listener
            .accept()
            .await
            .unwrap();
        let mut client = MockClient { stream };

        // Send auth request
        client
            .send_raw("Content-Type: auth/request\n\n")
            .await;

        // Read auth command
        let cmd = client
            .read_command()
            .await;
        let expected = format!("auth {}\n\n", self.password);
        if cmd == expected {
            client
                .reply_ok()
                .await;
        } else {
            client
                .reply_err("Invalid password")
                .await;
        }

        client
    }
}

impl MockClient {
    pub fn from_stream(stream: TcpStream) -> Self {
        Self { stream }
    }

    pub async fn send_raw(&mut self, data: &str) {
        self.stream
            .write_all(data.as_bytes())
            .await
            .unwrap();
    }

    /// Send a text/event-plain event with correct two-part wire format
    pub async fn send_event_plain(&mut self, event_name: &str, headers: &HashMap<String, String>) {
        let mut body = format!(
            "Event-Name: {}\n",
            percent_encode(event_name.as_bytes(), NON_ALPHANUMERIC)
        );
        for (key, value) in headers {
            body.push_str(&format!(
                "{}: {}\n",
                key,
                percent_encode(value.as_bytes(), NON_ALPHANUMERIC)
            ));
        }
        body.push('\n');

        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-plain\n\n",
            body.len()
        );
        self.send_raw(&format!("{}{}", envelope, body))
            .await;
    }

    /// Send a text/event-plain event with an inner body (e.g. BACKGROUND_JOB)
    pub async fn send_event_plain_with_body(
        &mut self,
        event_name: &str,
        headers: &HashMap<String, String>,
        event_body: &str,
    ) {
        let mut inner = format!(
            "Event-Name: {}\n",
            percent_encode(event_name.as_bytes(), NON_ALPHANUMERIC)
        );
        for (key, value) in headers {
            inner.push_str(&format!(
                "{}: {}\n",
                key,
                percent_encode(value.as_bytes(), NON_ALPHANUMERIC)
            ));
        }
        inner.push_str(&format!("Content-Length: {}\n", event_body.len()));
        inner.push('\n');
        inner.push_str(event_body);

        let envelope = format!(
            "Content-Length: {}\nContent-Type: text/event-plain\n\n",
            inner.len()
        );
        self.send_raw(&format!("{}{}", envelope, inner))
            .await;
    }

    /// Send a HEARTBEAT event with realistic headers
    pub async fn send_heartbeat(&mut self) {
        let mut headers = HashMap::new();
        headers.insert("Core-UUID".to_string(), "test-core-uuid".to_string());
        headers.insert("FreeSWITCH-Hostname".to_string(), "test-host".to_string());
        headers.insert("Event-Info".to_string(), "System Ready".to_string());
        headers.insert(
            "Up-Time".to_string(),
            "0 years, 0 days, 1 hour, 23 minutes".to_string(),
        );
        headers.insert("Session-Count".to_string(), "5".to_string());
        headers.insert("Max-Sessions".to_string(), "1000".to_string());
        headers.insert("Heartbeat-Interval".to_string(), "20".to_string());
        self.send_event_plain("HEARTBEAT", &headers)
            .await;
    }

    /// Send a disconnect notice
    pub async fn send_disconnect_notice(&mut self, message: &str) {
        let data = format!(
            "Content-Type: text/disconnect-notice\nContent-Disposition: disconnect\nContent-Length: {}\n\n{}",
            message.len(),
            message
        );
        self.send_raw(&data)
            .await;
    }

    /// Read a command from the client (reads until \n\n)
    pub async fn read_command(&mut self) -> String {
        let mut reader = BufReader::new(&mut self.stream);
        let mut result = String::new();

        loop {
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .await
                .unwrap();
            if n == 0 {
                break;
            }
            result.push_str(&line);
            if result.ends_with("\n\n") {
                break;
            }
        }

        result
    }

    /// Send a +OK command reply
    pub async fn reply_ok(&mut self) {
        self.send_raw("Content-Type: command/reply\nReply-Text: +OK accepted\n\n")
            .await;
    }

    /// Send a command reply with a specific Reply-Text (for correlation tests).
    pub async fn reply_ok_text(&mut self, text: &str) {
        let msg = format!("Content-Type: command/reply\nReply-Text: +OK {}\n\n", text);
        self.send_raw(&msg)
            .await;
    }

    /// Send an api/response with body
    pub async fn reply_api(&mut self, body: &str) {
        let data = format!(
            "Content-Type: api/response\nContent-Length: {}\n\n{}",
            body.len(),
            body
        );
        self.send_raw(&data)
            .await;
    }

    /// Send a -ERR command reply
    pub async fn reply_err(&mut self, text: &str) {
        let msg = format!("Content-Type: command/reply\nReply-Text: -ERR {}\n\n", text);
        self.send_raw(&msg)
            .await;
    }

    /// Send a command reply with a raw Reply-Text value (for getvar)
    pub async fn reply_raw_text(&mut self, text: &str) {
        let msg = format!("Content-Type: command/reply\nReply-Text: {}\n\n", text);
        self.send_raw(&msg)
            .await;
    }

    /// Send a connect response in the flat serialized format FreeSWITCH uses.
    ///
    /// FreeSWITCH's `connect` handler calls `switch_event_serialize()` which
    /// percent-encodes ALL header values and sends the result as a flat blob
    /// (no outer envelope wrapper). This is unlike normal `command/reply`
    /// responses which write literal `Content-Type: command/reply\n`.
    pub async fn send_connect_response(&mut self, channel_headers: &HashMap<String, String>) {
        let mut data = String::new();

        // Channel data headers first (like switch_channel_event_set_data).
        // switch_event_serialize(SWITCH_TRUE) percent-encodes every value.
        for (key, value) in channel_headers {
            data.push_str(&format!(
                "{}: {}\n",
                key,
                percent_encode(value.as_bytes(), NON_ALPHANUMERIC)
            ));
        }

        // Protocol headers last (like switch_event_add_header_string STACK_BOTTOM)
        let protocol_headers = [
            ("Content-Type", "command/reply"),
            ("Reply-Text", "+OK"),
            ("Socket-Mode", "async"),
            ("Control", "full"),
        ];
        for (key, value) in &protocol_headers {
            data.push_str(&format!(
                "{}: {}\n",
                key,
                percent_encode(value.as_bytes(), NON_ALPHANUMERIC)
            ));
        }
        data.push('\n');

        self.send_raw(&data)
            .await;
    }

    /// Drop the TCP connection
    pub async fn drop_connection(self) {
        drop(self.stream);
    }
}

/// Bind an ephemeral dual-stack listener and hand back the port to dial it on.
///
/// For the tests that drive the socket themselves -- outbound accept, re-exec
/// adoption, connection-refused -- rather than going through
/// [`MockEslServer`].
pub async fn setup_raw_pair() -> (TcpListener, u16) {
    let listener = TcpListener::bind("[::]:0")
        .await
        .expect("bind an ephemeral listener");
    let port = listener
        .local_addr()
        .expect("a bound listener has a local address")
        .port();
    (listener, port)
}

/// Create a connected mock pair (MockClient, EslClient, EslEventStream)
pub async fn setup_connected_pair(
    password: &str,
) -> (
    MockClient,
    freeswitch_esl_tokio::EslClient,
    freeswitch_esl_tokio::EslEventStream,
) {
    setup_connected_pair_with_options(password, freeswitch_esl_tokio::EslConnectOptions::default())
        .await
}

/// Create a connected mock pair with custom options
pub async fn setup_connected_pair_with_options(
    password: &str,
    options: freeswitch_esl_tokio::EslConnectOptions,
) -> (
    MockClient,
    freeswitch_esl_tokio::EslClient,
    freeswitch_esl_tokio::EslEventStream,
) {
    let server = MockEslServer::start(password).await;
    let port = server.port();

    let (mock_client, esl_result) = tokio::join!(
        server.accept(),
        freeswitch_esl_tokio::EslClient::connect_with_options("127.0.0.1", port, password, options)
    );

    let (esl_client, esl_events) = esl_result.unwrap();
    (mock_client, esl_client, esl_events)
}

/// Receive the next event with a 5s timeout, unwrapping every layer down to
/// the `EslEvent`. Panics with a descriptive message on timeout, channel
/// close, or event error -- the common case in tests that expect success.
pub async fn recv_event(
    events: &mut freeswitch_esl_tokio::EslEventStream,
) -> freeswitch_esl_tokio::EslEvent {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("timeout")
        .expect("channel closed")
        .expect("event error")
}

/// Shared scaffolding for the `inflight_command_woken_on_*` tests: spawn an
/// `api("status")` call, let the mock read it, then run `action` (the
/// differing disconnect/notice/rejection/desync trigger) and return the
/// error the in-flight command failed with.
///
/// `action` takes ownership of `mock` and hands it back as `Some` unless it
/// closed the connection itself (`drop_connection` consumes it) -- the
/// returned value is held alive until the join completes, so a no-op or
/// non-closing action doesn't drop the socket out from under a test that
/// depends on the connection staying nominally open (e.g. liveness expiry).
pub async fn inflight_command_woken_on<F, Fut>(
    client: &freeswitch_esl_tokio::EslClient,
    mut mock: MockClient,
    join_timeout: Duration,
    action: F,
) -> freeswitch_esl_tokio::EslError
where
    F: FnOnce(MockClient) -> Fut,
    Fut: std::future::Future<Output = Option<MockClient>>,
{
    let api_task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .api("status")
                .await
        }
    });

    mock.read_command()
        .await;
    let _mock = action(mock).await;

    tokio::time::timeout(join_timeout, api_task)
        .await
        .expect("in-flight command still blocked after the triggering action")
        .expect("api task panicked")
        .expect_err("command should fail after the triggering action")
}
