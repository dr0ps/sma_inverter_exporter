extern crate socket2;

use self::socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr};
use tokio::time::Duration;

/*
 *
 * by dr0ps 2020-Jul-18
 *
 *
 *  this software is released under GNU General Public License, version 2.
 *  This program is free software;
 *  you can redistribute it and/or modify it under the terms of the GNU General Public License
 *  as published by the Free Software Foundation; version 2 of the License.
 *  This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY;
 *  without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
 *  See the GNU General Public License for more details.
 *
 *  You should have received a copy of the GNU General Public License along with this program;
 *  if not, write to the Free Software Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301, USA.
 *
 *
 */


pub fn initialize_socket(multicast: bool, src_port: u16) -> Socket {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
    socket.set_reuse_address(true).unwrap();
    socket
        .bind(&SockAddr::from(SocketAddr::new(
            Ipv4Addr::new(0, 0, 0, 0).into(),
            src_port,
        )))
        .unwrap();
    match socket.set_read_timeout(Some(Duration::from_secs(1))) {
        Ok(()) => {}
        Err(error) => {
            println!("Unable to set socket timeout {}", error);
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
