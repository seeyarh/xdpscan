mod recv;
mod send;
use recv::recv;
use send::send;

use std::cmp;
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU32;
use std::sync::atomic::AtomicBool;
use std::time;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

use etherparse::PacketBuilder;
use xsk_rs::{
    socket::{Config as SocketConfig, *},
    umem::{Config as UmemConfig, *},
    BindFlags, LibbpfFlags, XdpFlags,
};

#[derive(Debug)]
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
    let fill_q_size: u32 = 4096 * 2;
    let frame_size: u32 = 2048;
    let max_batch_size: usize = 64;
    let num_frames_to_send: usize = 64;
    let poll_ms_timeout: i32 = 100;
    let frame_count = rx_q_size + tx_q_size;
    let wait_time_secs = 8;

    let umem_config = UmemConfig::new(
        NonZeroU32::new(frame_count).unwrap(),
        NonZeroU32::new(frame_size).unwrap(),
        fill_q_size,
        comp_q_size,
        0,
        false,
    )
    .unwrap();

    let (mut umem, fq, cq, frames) = Umem::builder(umem_config)
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

    let queue_id = 0;
    let (tx_q, rx_q) =
        Socket::new(socket_config, &mut umem, ifname, queue_id).expect("failed to build socket");

    let n_tx_frames = frames.len() / 2;

    let (tx_umem, rx_umem, tx_frames, rx_frames) = umem.split(frames, n_tx_frames as usize);

    let tx_done = Arc::new(AtomicBool::new(false));
    let rx_done = tx_done.clone();

    let recv_handle = thread::spawn(|| recv(rx_q, fq, rx_frames, rx_umem, rx_done));
    thread::sleep(time::Duration::from_secs(1));
    let send_handle = thread::spawn(|| {
        send(targets, src_config, tx_q, cq, tx_frames, tx_umem);
    });

    send_handle.join().unwrap();

    thread::sleep(time::Duration::from_secs(wait_time_secs));
    tx_done.store(true, Ordering::Relaxed);

    let responders = recv_handle.join().unwrap();
    eprintln!("{:?}", responders);
    for responder in responders {
        eprintln!("{:?}", responder);
    }
}
