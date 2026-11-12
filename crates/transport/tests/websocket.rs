// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

#[cfg(test)]
mod tests {
    use ccp_transport::{
        ByteStream,
        websocket::{
            WebSocketStream,
            tungstenite::{Message, protocol::Role},
        },
        websocket_config,
    };
    use futures_util::{SinkExt, StreamExt};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
        time::{Duration, timeout},
    };
    async fn pair(cap: usize) -> (WebSocketStream<DuplexStream>, WebSocketStream<DuplexStream>) {
        let (a, b) = tokio::io::duplex(cap);
        tokio::join!(
            WebSocketStream::from_raw_socket(a, Role::Client, Some(websocket_config())),
            WebSocketStream::from_raw_socket(b, Role::Server, Some(websocket_config()))
        )
    }
    #[tokio::test]
    async fn empty_nonfinal_fragments_are_consumed_within_the_message_payload_cap() {
        use ccp_transport::websocket::tungstenite::protocol::frame::{
            Frame,
            coding::{Data, OpCode},
        };
        let (mut sender, receiver) = pair(8192).await;
        let mut reader = ByteStream::new(receiver);
        sender
            .send(Message::Frame(Frame::message(
                Vec::new(),
                OpCode::Data(Data::Binary),
                false,
            )))
            .await
            .unwrap();
        for _ in 0..301 {
            sender
                .send(Message::Frame(Frame::message(
                    Vec::new(),
                    OpCode::Data(Data::Continue),
                    false,
                )))
                .await
                .unwrap();
        }
        sender
            .send(Message::Frame(Frame::message(
                vec![42],
                OpCode::Data(Data::Continue),
                true,
            )))
            .await
            .unwrap();
        let mut byte = [0];
        timeout(Duration::from_secs(1), reader.read_exact(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(byte, [42]);
    }
    #[tokio::test]
    async fn simultaneous_bulk_duplex_progresses_without_flush_read_deadlock() {
        let (a, b) = pair(8192).await;
        let (mut ar, mut aw) = tokio::io::split(ByteStream::new(a));
        let (mut br, mut bw) = tokio::io::split(ByteStream::new(b));
        let a = vec![41; 2 * 1024 * 1024];
        let b = vec![72; 2 * 1024 * 1024];
        let mut ra = vec![0; b.len()];
        let mut rb = vec![0; a.len()];
        timeout(Duration::from_secs(3), async {
            let (wa, wb, ra, rb) = tokio::join!(
                async {
                    aw.write_all(&a).await?;
                    aw.flush().await
                },
                async {
                    bw.write_all(&b).await?;
                    bw.flush().await
                },
                ar.read_exact(&mut ra),
                br.read_exact(&mut rb)
            );
            wa.unwrap();
            wb.unwrap();
            ra.unwrap();
            rb.unwrap();
        })
        .await
        .unwrap();
        assert_eq!(ra, b);
        assert_eq!(rb, a);
    }
    #[tokio::test]
    async fn unread_peer_backpressures_without_unbounded_buffers() {
        let (a, _unread) = pair(8192).await;
        let mut a = ByteStream::new(a);
        assert!(
            timeout(
                Duration::from_millis(100),
                a.write_all(&vec![0; 1024 * 1024])
            )
            .await
            .is_err()
        );
    }
    #[tokio::test]
    async fn text_oversized_single_and_unfinished_fragmented_frames_refuse() {
        use ccp_transport::websocket::tungstenite::protocol::frame::{
            Frame,
            coding::{Data, OpCode},
        };
        for case in 0..3 {
            let (a, b) = tokio::io::duplex(131072);
            let mut a = WebSocketStream::from_raw_socket(a, Role::Client, None).await;
            let b =
                WebSocketStream::from_raw_socket(b, Role::Server, Some(websocket_config())).await;
            let mut b = ByteStream::new(b);
            match case {
                0 => a.send(Message::Text("refuse".into())).await.unwrap(),
                1 => a
                    .send(Message::Binary(vec![0; 65537].into()))
                    .await
                    .unwrap(),
                _ => {
                    a.send(Message::Frame(Frame::message(
                        vec![0; 32768],
                        OpCode::Data(Data::Binary),
                        false,
                    )))
                    .await
                    .unwrap();
                    a.send(Message::Frame(Frame::message(
                        vec![0; 32769],
                        OpCode::Data(Data::Continue),
                        false,
                    )))
                    .await
                    .unwrap();
                }
            }
            let error = timeout(Duration::from_secs(1), b.read(&mut [0; 1]))
                .await
                .unwrap()
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(if case == 0 { "binary" } else { "Space limit" })
            );
        }
    }
    #[tokio::test(start_paused = true)]
    async fn keepalives_survive_across_windows_and_ping_empty_floods_are_bounded() {
        for empty in [false, true] {
            let (mut a, b) = pair(8192).await;
            let mut b = ByteStream::new(b);
            for _ in 0..3 {
                for _ in 0..100 {
                    a.send(if empty {
                        Message::Binary(Vec::new().into())
                    } else {
                        Message::Ping(vec![1].into())
                    })
                    .await
                    .unwrap();
                }
                a.send(Message::Binary(vec![9].into())).await.unwrap();
                let mut byte = [0];
                b.read_exact(&mut byte).await.unwrap();
                assert_eq!(byte, [9]);
                if !empty {
                    assert!(matches!(a.next().await.unwrap().unwrap(), Message::Pong(_)));
                }
                tokio::time::advance(Duration::from_secs(2)).await;
            }
            for _ in 0..129 {
                a.send(if empty {
                    Message::Binary(Vec::new().into())
                } else {
                    Message::Ping(vec![1].into())
                })
                .await
                .unwrap();
            }
            assert!(
                b.read(&mut [0; 1])
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("rate exceeded")
            );
        }
    }
    #[tokio::test]
    async fn close_empty_io_partial_reads_and_write_shutdown() {
        let (a, mut b) = pair(8192).await;
        let mut a = ByteStream::new(a);
        assert_eq!(a.read(&mut []).await.unwrap(), 0);
        assert_eq!(a.write(&[]).await.unwrap(), 0);
        b.send(Message::Binary(vec![1, 2, 3].into())).await.unwrap();
        let mut data = [0; 1];
        a.read_exact(&mut data).await.unwrap();
        assert_eq!(data, [1]);
        let mut rest = [0; 2];
        a.read_exact(&mut rest).await.unwrap();
        assert_eq!(rest, [2, 3]);
        b.close(None).await.unwrap();
        assert_eq!(a.read(&mut [0; 1]).await.unwrap(), 0);
        assert_eq!(a.read(&mut [0; 1]).await.unwrap(), 0);
        let (a, mut b) = pair(8192).await;
        let mut a = ByteStream::new(a);
        a.shutdown().await.unwrap();
        assert!(matches!(
            b.next().await.unwrap().unwrap(),
            Message::Close(_)
        ));
    }
}
