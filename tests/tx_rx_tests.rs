extern crate utilities;
use std::error::Error;
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU32;
use utilities::veth_setup::{cleanup_veth, setup_veth, LinkIpAddr, VethConfig, VethLink};

use etherparse::PacketBuilder;
use serial_test::serial;

use xsk_rs::{
    socket::{Config as SocketConfig, *},
    umem::{Config as UmemConfig, *},
    BindFlags, LibbpfFlags, XdpFlags,
};

use std::thread;
use std::time::Duration;

use xdpscan::{SrcConfig, Target};

fn check_packet_match(
    value: &etherparse::PacketHeaders,
    expected_src: Ipv4Addr,
    expected_dst: Ipv4Addr,
) -> bool {
    if let Some(ip) = &value.ip {
        if let etherparse::IpHeader::Version4(ipv4) = &ip {
            println!("received frame in test receiver");
            println!("link: {:?}", value.link);
            println!("ip: {:?}", &value.ip);
            println!("transport: {:?}", value.transport);
            return (ipv4.source == expected_src.octets())
                && (ipv4.destination == expected_dst.octets());
        }
    }
    return false;
}

fn generate_synack_frame_resp(value: etherparse::PacketHeaders) -> Vec<u8> {
    let link_layer = value.link.unwrap();
    let ip = value.ip.unwrap();
    let tcp = value.transport.unwrap().tcp().unwrap();
    let ipv4 = match ip {
        etherparse::IpHeader::Version4(ipv4) => ipv4,
        _ => panic!("err"),
    };

    let builder = PacketBuilder::ethernet2(link_layer.destination, link_layer.source)
        .ipv4(ipv4.destination, ipv4.source, 20)
        .tcp(tcp.destination_port, tcp.source_port, 0, 0)
        .syn()
        .ack(0);
    let mut result = Vec::<u8>::with_capacity(builder.size(0));
    builder.write(&mut result, &[]).unwrap();
    result
}

fn recv(
    mut fill_q: xsk_rs::FillQueue,
    mut _comp_q: xsk_rs::CompQueue,
    mut tx_q: xsk_rs::TxQueue,
    mut rx_q: xsk_rs::RxQueue,
    mut frames: Vec<xsk_rs::FrameDesc>,
    mut umem: xsk_rs::Umem,
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
) {
    // 1. Add frames to dev2's FillQueue
    //assert_eq!(unsafe { fill_q.produce(&frames[..]) }, frames.len());
    unsafe { fill_q.produce(&frames[1..2]) };
    eprintln!("--------------------------");
    eprintln!(
        "test receiver rx frames[0] = {}, rx frames[-1] = {}",
        frames[0].addr(),
        frames[frames.len() - 1].addr()
    );
    eprintln!("--------------------------");

    let mut total_frames_rcvd = 0;
    let mut matching_frames_rcvd = 0;
    let poll_ms_timeout: i32 = 100;
    let num_frames_sent = 1;

    while matching_frames_rcvd < num_frames_sent {
        eprintln!("starting rx loop");
        match rx_q
            .poll_and_consume(&mut frames[..], poll_ms_timeout)
            .unwrap()
        {
            0 => {
                eprintln!("(dev) dev.rx_q.poll_and_consume() consumed 0 frames");
                // No packets consumed, wake up fill queue if required
                if fill_q.needs_wakeup() {
                    eprintln!("(dev) waking up dev.fill_q");
                    fill_q.wakeup(rx_q.fd(), poll_ms_timeout).unwrap();
                }
            }
            frames_rcvd => {
                eprintln!(
                    "(dev) dev.rx_q.poll_and_consume() consumed {} frames",
                    frames_rcvd
                );

                let mut resp_frames = vec![];
                for recv_frame in frames.iter().take(frames_rcvd) {
                    let frame_ref = unsafe {
                        umem.read_from_umem_checked(&recv_frame.addr(), &recv_frame.len())
                            .unwrap()
                    };
                    eprintln!(
                        "\ntest receiver: recv frame addr = {}, len = {}\n",
                        recv_frame.addr(),
                        recv_frame.len()
                    );

                    match etherparse::PacketHeaders::from_ethernet_slice(&frame_ref) {
                        Err(value) => println!("Err {:?}", value),
                        Ok(value) => {
                            if check_packet_match(&value, src_ip, dst_ip) {
                                matching_frames_rcvd += 1;
                                let resp_frame = generate_synack_frame_resp(value);
                                resp_frames.push(resp_frame);
                            }
                        }
                    }
                }

                for resp_frame in resp_frames {
                    eprintln!("test receiver resp frame len = {}", resp_frame.len());
                    unsafe {
                        umem.write_to_umem_checked(&mut frames[0], &resp_frame)
                            .unwrap();
                    };
                    unsafe {
                        tx_q.produce_and_wakeup(&frames[..1]).unwrap();
                    };
                }
                // Add frames back to fill queue
                while unsafe {
                    fill_q
                        .produce_and_wakeup(&frames[..frames_rcvd], rx_q.fd(), poll_ms_timeout)
                        .unwrap()
                } != frames_rcvd
                {
                    // Loop until frames added to the fill ring.
                    eprintln!("(dev) dev.fill_q.produce_and_wakeup() failed to allocate");
                }

                eprintln!(
                    "(dev) dev.fill_q.produce_and_wakeup() submitted {} frames",
                    frames_rcvd
                );

                total_frames_rcvd += frames_rcvd;
                eprintln!("(dev) total frames received: {}", total_frames_rcvd);
            }
        }
    }
}

fn run_test(veth_link: &VethLink) -> Result<(), Box<dyn Error>> {
    let rx_q_size: u32 = 4096;
    let tx_q_size: u32 = 4096;
    let comp_q_size: u32 = 4096;
    let fill_q_size: u32 = 4096;
    let frame_size: u32 = 2048;
    let frame_count = 1024;
    let poll_ms_timeout: i32 = 100;
    let queue_id = 0;

    let umem_config = UmemConfig::new(
        NonZeroU32::new(frame_count).unwrap(),
        NonZeroU32::new(frame_size).unwrap(),
        fill_q_size,
        comp_q_size,
        0,
        false,
    )
    .unwrap();

    let (mut umem, fill_q, comp_q, frames) = Umem::builder(umem_config)
        .create_mmap()
        .expect("failed to create mmap area")
        .create_umem()
        .expect("failed to create umem");

    let socket_config = SocketConfig::new(
        rx_q_size,
        tx_q_size,
        LibbpfFlags::empty(),
        XdpFlags::empty(),
        BindFlags::XDP_USE_NEED_WAKEUP,
    )
    .unwrap();

    let (tx_q, rx_q) = Socket::new(socket_config, &mut umem, &veth_link.dev2_if_name, queue_id)
        .expect("failed to build socket");

    let src_mac = [0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a];
    let dst_mac = [0x4a, 0xf1, 0x30, 0xeb, 0x0d, 0x31];
    let src_ipv4 = Ipv4Addr::new(192, 168, 69, 1);
    let src_ip = IpAddr::V4(src_ipv4);
    let src_port = 4321;
    let dst_ipv4 = Ipv4Addr::new(192, 168, 69, 2);
    let dst_ip = IpAddr::V4(dst_ipv4);

    let recv_handle =
        thread::spawn(move || recv(fill_q, comp_q, tx_q, rx_q, frames, umem, src_ipv4, dst_ipv4));

    let targets = vec![Target {
        ip: dst_ip,
        port: 1234,
    }];

    let src_config = SrcConfig {
        src_mac,
        dst_mac,
        src_ip,
        src_port,
    };
    xdpscan::start_scan(&veth_link.dev1_if_name, src_config, targets);

    recv_handle.join().expect("failed to join recv handle");
    Ok(())
}

#[test]
fn tx_rx_test() -> Result<(), Box<dyn Error>> {
    let veth_config = VethConfig::new(
        "veth0".into(),
        "veth1".into(),
        [0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a],
        [0x4a, 0xf1, 0x30, 0xeb, 0x0d, 0x31],
        LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 1), 24),
        LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 2), 24),
    );
    let (veth_link, mut rt) = setup_veth(&veth_config);
    let passed = run_test(&veth_link);
    cleanup_veth(&veth_link, &mut rt);
    assert!(false);
    passed
}
