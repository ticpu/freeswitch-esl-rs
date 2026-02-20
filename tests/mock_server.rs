//! Mock FreeSWITCH ESL server for integration testing

use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use std::collections::HashMap;
use std::net::SocketAddr;
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

    /// Drop the TCP connection
    pub async fn drop_connection(self) {
        drop(self.stream);
    }
}

/// Create a connected mock pair (MockClient, EslClient, EslEventStream)
pub async fn setup_connected_pair(
    password: &str,
) -> (
    MockClient,
    freeswitch_esl_tokio::EslClient,
    freeswitch_esl_tokio::EslEventStream,
) {
    let server = MockEslServer::start(password).await;
    let port = server.port();

    let (mock_client, esl_result) = tokio::join!(
        server.accept(),
        freeswitch_esl_tokio::EslClient::connect("127.0.0.1", port, password)
    );

    let (esl_client, esl_events) = esl_result.unwrap();
    (mock_client, esl_client, esl_events)
}
