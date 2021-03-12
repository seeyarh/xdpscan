use crate::{SrcConfig, Target};
use etherparse::PacketBuilder;
use std::net::{IpAddr, Ipv4Addr};
use xsk_rs::{CompQueue, FrameDesc, TxQueue, Umem};

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

pub fn send(
    targets: Vec<Target>,
    src_config: SrcConfig,
    mut tx_q: TxQueue,
    mut _comp_q: CompQueue,
    mut frame_descs: Vec<FrameDesc>,
    mut umem: Umem,
) {
    for (i, target) in targets.iter().enumerate() {
        // Copy over some bytes to devs umem to transmit
        let eth_frame = generate_eth_frame(
            src_config.src_mac,
            src_config.dst_mac,
            src_config.src_ip,
            src_config.src_port,
            target.ip,
            target.port,
        );

        unsafe {
            umem.write_to_umem_checked(&mut frame_descs[i], &eth_frame)
                .unwrap()
        };

        assert_eq!(frame_descs[0].len(), eth_frame.len());
    }

    // 3. Hand over the frame to the kernel for transmission
    assert_eq!(
        unsafe {
            tx_q.produce_and_wakeup(&frame_descs[..targets.len()])
                .unwrap()
        },
        1
    );
}
