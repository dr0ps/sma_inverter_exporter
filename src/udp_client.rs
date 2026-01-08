extern crate socket2;

use self::socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr};
use tokio::time::Duration;

use crate::log;

pub fn initialize_socket(multicast: bool) -> Socket {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
    socket.set_reuse_address(true).unwrap();
    socket
        .bind(&SockAddr::from(SocketAddr::new(
            Ipv4Addr::new(0, 0, 0, 0).into(),
            9522,
        )))
        .unwrap();
    match socket.set_read_timeout(Some(Duration::from_secs(1))) {
        Ok(()) => {}
        Err(error) => {
            log!(format!("Unable to set socket timeout {}", error));
        }
    }

    if multicast {
        assert!(socket
            .join_multicast_v4(
                &Ipv4Addr::new(239, 12, 255, 254),
                &Ipv4Addr::new(0, 0, 0, 0)
            )
            .is_ok());
    }
    socket
}
