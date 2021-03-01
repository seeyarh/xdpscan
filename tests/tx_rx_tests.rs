extern crate utilities;
use std::net::Ipv4Addr;
use std::num::NonZeroU32;
use utilities::veth_setup::{cleanup_veth, setup_veth, LinkIpAddr, VethConfig, VethLink};

use etherparse::PacketBuilder;
use serial_test::serial;
use xsk_rs::{
    socket::{Config as SocketConfig, ConfigBuilder as SocketConfigBuilder},
    umem::{Config as UmemConfig, ConfigBuilder as UmemConfigBuilder},
    xsk::{build_socket_and_umem, Xsk},
    BindFlags, LibbpfFlags, XdpFlags,
};

use std::thread;
use std::time::Duration;

fn run_test(veth_link: &VethLink) {
    let rx_q_size: u32 = 4096;
    let tx_q_size: u32 = 4096;
    let comp_q_size: u32 = 4096;
    let fill_q_size: u32 = 4096;
    let frame_size: u32 = 2048;
    let frame_count = 1024;
    let poll_ms_timeout: i32 = 100;

    let umem_config = UmemConfig::new(
        NonZeroU32::new(frame_count).unwrap(),
        NonZeroU32::new(frame_size).unwrap(),
        fill_q_size,
        comp_q_size,
        0,
        false,
    )
    .unwrap();

    let socket_config = SocketConfig::new(
        rx_q_size,
        tx_q_size,
        LibbpfFlags::empty(),
        XdpFlags::empty(),
        BindFlags::XDP_USE_NEED_WAKEUP,
    )
    .unwrap();

    xdpscan::run_scan(&veth_link.dev1_if_name);

    let mut dev2 = build_socket_and_umem(umem_config, socket_config, &veth_link.dev2_if_name, 0);
    let mut dev2_frames = dev2.frame_descs;

    // 1. Add frames to dev2's FillQueue
    assert_eq!(
        unsafe { dev2.fill_q.produce(&dev2_frames[..]) },
        dev2_frames.len()
    );

    // 4. Read from dev2
    let packets_recvd = dev2
        .rx_q
        .poll_and_consume(&mut dev2_frames[..], 10)
        .unwrap();

    // Check that one of the packets we received matches what we expect.
    for recv_frame in dev2_frames.iter().take(packets_recvd) {
        let frame_ref = unsafe {
            dev2.umem
                .read_from_umem_checked(&recv_frame.addr(), &recv_frame.len())
                .unwrap()
        };

        match etherparse::PacketHeaders::from_ethernet_slice(&frame_ref) {
            Err(value) => println!("Err {:?}", value),
            Ok(value) => {
                println!("recvd frame");
                println!("link: {:?}", value.link);
                println!("ip: {:?}", value.ip);
                println!("transport: {:?}", value.transport);
            }
        }
    }
}

#[test]
fn tx_rx_test() {
    let veth_config = VethConfig::new(
        "veth0".into(),
        "veth1".into(),
        [0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a],
        [0x4a, 0xf1, 0x30, 0xeb, 0x0d, 0x31],
        LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 1), 24),
        LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 2), 24),
    );
    let (veth_link, mut rt) = setup_veth(&veth_config);
    run_test(&veth_link);
    //cleanup_veth(&veth_link, &mut rt);
    assert_eq!(1, 2);
}
