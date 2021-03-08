mod recv;
mod send;

use std::cmp;
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU32;
use std::thread;

use etherparse::PacketBuilder;
use xsk_rs::{
    socket, socket::Config as SocketConfig, umem::Config as UmemConfig, xsk::build_socket_and_umem,
    BindFlags, LibbpfFlags, XdpFlags,
};

pub struct Target {
    pub ip: IpAddr,
    pub port: u16,
}

fn generate_eth_frame(
    src_mac: [u8; 6],
    dst_mac: [u8; 6],
    src_ip: IpAddr,
    src_port: u16,
    dst_ip: IpAddr,
    dst_port: u16,
) -> Vec<u8> {
    let builder = PacketBuilder::ethernet2(src_mac, dst_mac);

    if src_ip.is_ipv6() || dst_ip.is_ipv6() {
        let src_ip = match src_ip {
            IpAddr::V4(ipv4) => ipv4.to_ipv6_mapped(),
            IpAddr::V6(ipv6) => ipv6,
        };
        let dst_ip = match dst_ip {
            IpAddr::V4(ipv4) => ipv4.to_ipv6_mapped(),
            IpAddr::V6(ipv6) => ipv6,
        };

        let builder = builder
            .ipv6(src_ip.octets(), dst_ip.octets(), 20)
            .tcp(src_port, dst_port, 0, 4)
            .syn();
        let mut result = Vec::<u8>::with_capacity(builder.size(0));
        builder.write(&mut result, &[]).unwrap();
        result
    } else {
        let src_ip = match src_ip {
            IpAddr::V4(ipv4) => ipv4,
            IpAddr::V6(ipv6) => ipv6
                .to_ipv4()
                .expect("Failed to convert ipv6 to ipv4, this shouldn't happen"),
        };
        let dst_ip = match dst_ip {
            IpAddr::V4(ipv4) => ipv4,
            IpAddr::V6(ipv6) => ipv6
                .to_ipv4()
                .expect("Failed to convert ipv6 to ipv4, this shouldn't happen"),
        };
        let builder = builder
            .ipv4(
                src_ip.octets(), // src ip
                dst_ip.octets(), // dst ip
                20,              // time to live
            )
            .tcp(src_port, dst_port, 0, 4)
            .syn();
        let mut result = Vec::<u8>::with_capacity(builder.size(0));
        builder.write(&mut result, &[]).unwrap();
        result
    }
}

pub fn run_scan(
    ifname: &str,
    src_mac: [u8; 6],
    dst_mac: [u8; 6],
    src_ip: IpAddr,
    src_port: u16,
    targets: Vec<Target>,
) {
    let rx_q_size: u32 = 4096;
    let tx_q_size: u32 = 4096;
    let comp_q_size: u32 = 4096;
    let fill_q_size: u32 = 4096 * 4;
    let frame_size: u32 = 2048;
    let max_batch_size: usize = 64;
    let num_frames_to_send: usize = 64;
    let poll_ms_timeout: i32 = 100;
    let frame_count = fill_q_size + comp_q_size;

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

    let mut dev = build_socket_and_umem(umem_config, socket_config, &ifname, 0);

    // Populate tx queue
    let frames = &mut dev.frame_descs;

    for (i, target) in targets.iter().enumerate() {
        // Copy over some bytes to devs umem to transmit
        let eth_frame =
            generate_eth_frame(src_mac, dst_mac, src_ip, src_port, target.ip, target.port);

        unsafe {
            dev.umem
                .write_to_umem_checked(&mut frames[i], &eth_frame)
                .unwrap()
        };

        assert_eq!(frames[0].len(), eth_frame.len());
    }

    // 3. Hand over the frame to the kernel for transmission
    assert_eq!(
        unsafe {
            dev.tx_q
                .produce_and_wakeup(&frames[..targets.len()])
                .unwrap()
        },
        1
    );
}
