mod recv;
mod send;
use recv::recv;
use send::send;

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

pub struct SrcConfig {
    pub src_mac: [u8; 6],
    pub dst_mac: [u8; 6],
    pub src_ip: IpAddr,
    pub src_port: u16,
}

pub fn start_scan(ifname: &str, src_config: SrcConfig, targets: Vec<Target>) {
    let rx_q_size: u32 = 4096;
    let tx_q_size: u32 = 4096;
    let comp_q_size: u32 = 4096;
    let fill_q_size: u32 = 4096;
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

    let dev = build_socket_and_umem(umem_config, socket_config, &ifname, 0);

    let n_tx_frames = dev.frame_descs.len() / 2;
    let tx_frames = dev.frame_descs[..n_tx_frames].into();
    let rx_frames = dev.frame_descs[n_tx_frames..].into();

    let tx_q = dev.tx_q;
    let rx_q = dev.rx_q;
    let comp_q = dev.comp_q;
    let fill_q = dev.fill_q;

    let tx_umem = dev.umem;
    let rx_umem = tx_umem.clone();

    let send_handle = thread::spawn(|| {
        send(targets, src_config, tx_q, comp_q, tx_frames, tx_umem);
    });

    let recv_handle = thread::spawn(|| {
        recv(rx_q, fill_q, rx_frames, rx_umem);
    });

    send_handle.join().unwrap();
    recv_handle.join().unwrap();
}
