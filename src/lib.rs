mod recv;
mod send;

use std::cmp;
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU32;

use etherparse::PacketBuilder;
use xsk_rs::{
    socket, socket::Config as SocketConfig, umem::Config as UmemConfig, xsk::build_socket_and_umem,
    BindFlags, LibbpfFlags, XdpFlags,
};

fn generate_eth_frame(
    src_mac: [u8; 6],
    dst_mac: [u8; 6],
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
) -> Vec<u8> {
    let builder = PacketBuilder::ethernet2(src_mac, dst_mac)
        .ipv4(
            src_ip.octets(), // src ip
            dst_ip.octets(), // dst ip
            20,              // time to live
        )
        .udp(
            1234, // src port
            1234, // dst port
        );

    let mut result = Vec::<u8>::with_capacity(builder.size(0));

    builder.write(&mut result, &[]).unwrap();

    result
}

pub fn run_scan(ifname: &str) {
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

    // Copy over some bytes to devs umem to transmit
    let eth_frame = generate_eth_frame(
        [0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a],
        [0x4a, 0xf1, 0x30, 0xeb, 0x0d, 0x31],
        Ipv4Addr::new(192, 168, 69, 1),
        Ipv4Addr::new(192, 168, 69, 2),
    );

    match etherparse::PacketHeaders::from_ethernet_slice(&eth_frame) {
        Err(value) => println!("Err {:?}", value),
        Ok(value) => {
            println!("link: {:?}", value.link);
            println!("ip: {:?}", value.ip);
            println!("transport: {:?}", value.transport);
        }
    }

    unsafe {
        dev.umem
            .write_to_umem_checked(&mut frames[0], &eth_frame)
            .unwrap()
    };

    assert_eq!(frames[0].len(), eth_frame.len());

    // 3. Hand over the frame to the kernel for transmission
    assert_eq!(
        unsafe { dev.tx_q.produce_and_wakeup(&frames[..1]).unwrap() },
        1
    );
}
