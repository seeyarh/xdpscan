use crate::Target;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::Ordering;
use std::sync::{atomic::AtomicBool, Arc};
use xsk_rs::{FillQueue, FrameDesc, RxQueue, Umem};

fn parse_response(value: &etherparse::PacketHeaders) -> Option<Target> {
    let ip_hdr = value.ip.as_ref()?;
    eprintln!("parse: ip_hdr = {:?}", ip_hdr);
    let src_ip = match ip_hdr {
        etherparse::IpHeader::Version4(ipv4_hdr) => IpAddr::V4(ipv4_hdr.source.into()),
        etherparse::IpHeader::Version6(ipv6_hdr) => IpAddr::V6(ipv6_hdr.destination.into()),
    };

    let transport_hdr = value.transport.as_ref()?;
    eprintln!("parse: transport_hdr = {:?}", transport_hdr);
    let src_port = match transport_hdr {
        etherparse::TransportHeader::Udp(udp_hdr) => return None,
        etherparse::TransportHeader::Tcp(tcp_hdr) => tcp_hdr.source_port,
    };

    Some(Target {
        ip: src_ip,
        port: src_port.into(),
    })
}

pub fn recv(
    mut rx_q: RxQueue,
    mut fill_q: FillQueue,
    mut frames: Vec<FrameDesc>,
    umem: Umem,
    done: Arc<AtomicBool>,
) -> Vec<Target> {
    eprintln!("--------------------------");
    eprintln!(
        "rx frames[0] = {}, rx frames[-1] = {}",
        frames[0].addr(),
        frames[frames.len() - 1].addr()
    );
    eprintln!("--------------------------");
    assert_eq!(unsafe { fill_q.produce(&frames[..]) }, frames.len());

    let poll_ms_timeout: i32 = 100;
    let mut total_frames_rcvd = 0;

    let mut responders = vec![];

    while !(done.load(Ordering::Relaxed)) {
        eprintln!("starting rx loop");
        match rx_q
            .poll_and_consume(&mut frames[..], poll_ms_timeout)
            .unwrap()
        {
            0 => {
                eprintln!("rx_q.poll_and_consume() consumed 0 frames");
                // No packets consumed, wake up fill queue if required
                if fill_q.needs_wakeup() {
                    eprintln!("(waking up fill_q");
                    fill_q.wakeup(rx_q.fd(), poll_ms_timeout).unwrap();
                }
            }
            frames_rcvd => {
                eprintln!("rx_q.poll_and_consume() consumed {} frames", frames_rcvd);

                for recv_frame in frames.iter().take(frames_rcvd) {
                    eprintln!(
                        "recv frame addr = {}, len = {}",
                        recv_frame.addr(),
                        recv_frame.len()
                    );
                    let frame_ref = unsafe {
                        umem.read_from_umem_checked(&recv_frame.addr(), &recv_frame.len())
                            .unwrap()
                    };

                    match etherparse::PacketHeaders::from_ethernet_slice(&frame_ref) {
                        Err(value) => println!("Err {:?}", value),
                        Ok(value) => {
                            eprintln!("received frame in xdpscan rx loop:");
                            eprintln!("{:?}", value);
                            if let Some(responder) = parse_response(&value) {
                                responders.push(responder);
                            }
                        }
                    }
                }

                // Add frames back to fill queue
                while unsafe {
                    fill_q
                        .produce_and_wakeup(&frames[..frames_rcvd], rx_q.fd(), poll_ms_timeout)
                        .unwrap()
                } != frames_rcvd
                {
                    // Loop until frames added to the fill ring.
                    eprintln!("fill_q.produce_and_wakeup() failed to allocate");
                }

                eprintln!(
                    "fill_q.produce_and_wakeup() submitted {} frames",
                    frames_rcvd
                );

                total_frames_rcvd += frames_rcvd;
                eprintln!("total frames received: {}", total_frames_rcvd);
            }
        }
    }

    responders
}
