//! The fake server's own rules.

use std::time::Duration;

use eidolon_bidi::frame::{self, Opcode};
use eidolon_bidi::{handshake, Error};
use eidolon_testkit::FakeBidi;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::timeout;

#[tokio::test]
async fn it_rejects_a_client_frame_that_is_not_masked() {
    let (mut client, server) = tokio::io::duplex(4096);
    let fake = FakeBidi::new();
    let page = fake.page();
    let served = tokio::spawn(fake.serve(server));

    let key = handshake::new_key().unwrap();
    client.write_all(handshake::request("/session", "127.0.0.1:1", &key).as_bytes()).await.unwrap();
    let (head, _) = handshake::read_head(&mut client).await.unwrap();
    handshake::check_response(&head, &key).unwrap();

    // A correct client masks; this one does not.
    let bytes = frame::encode(true, Opcode::Text, br#"{"id":1,"method":"session.status","params":{}}"#, None);
    client.write_all(&bytes).await.unwrap();

    let ended = timeout(Duration::from_secs(5), served).await.unwrap().unwrap();
    assert!(matches!(ended, Err(Error::Protocol(ref m)) if m.contains("unmasked")), "{ended:?}");
    assert!(page.rejected_unmasked());
    // It closes with 1002 (protocol error) and never answers the command.
    let mut rest = Vec::new();
    client.read_to_end(&mut rest).await.unwrap();
    let (close, _) = frame::parse(&rest).unwrap().unwrap();
    assert_eq!(close.opcode, Opcode::Close);
    assert_eq!(close.payload[..2], 1002u16.to_be_bytes());
    assert_eq!(page.methods().len(), 0);
}

#[tokio::test]
async fn it_does_not_mask_its_own_frames() {
    let (mut client, server) = tokio::io::duplex(4096);
    tokio::spawn(FakeBidi::new().ping_first().serve(server));
    let key = handshake::new_key().unwrap();
    client.write_all(handshake::request("/session", "127.0.0.1:1", &key).as_bytes()).await.unwrap();
    let (_, mut rest) = handshake::read_head(&mut client).await.unwrap();
    let frame = loop {
        if let Some((frame, _)) = frame::parse(&rest).unwrap() {
            break frame;
        }
        let mut chunk = [0u8; 64];
        let n = client.read(&mut chunk).await.unwrap();
        rest.extend_from_slice(&chunk[..n]);
    };
    assert_eq!(frame.opcode, Opcode::Ping);
    assert!(!frame.masked);
}
