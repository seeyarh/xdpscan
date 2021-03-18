use xsk_rs::{FillQueue, FrameDesc, RxQueue, Umem};

pub fn recv(mut rx_q: RxQueue, mut fill_q: FillQueue, mut frames: Vec<FrameDesc>, umem: Umem) {
    eprintln!("--------------------------");
    eprintln!(
        "rx frames[0] = {}, rx frames[-1] = {}",
        frames[0].addr(),
        frames[frames.len() - 1].addr()
    );
    eprintln!("--------------------------");
    //assert_eq!(unsafe { fill_q.produce(&frames[..]) }, frames.len());
    unsafe { fill_q.produce(&frames[..1]) };
    let mut done = false;

    let poll_ms_timeout: i32 = 100;
    let mut total_frames_rcvd = 0;

    while !done {
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
                    let frame_ref = unsafe {
                        umem.read_from_umem_checked(&recv_frame.addr(), &recv_frame.len())
                            .unwrap()
                    };
                    eprintln!(
                        "recv frame addr = {}, len = {}",
                        recv_frame.addr(),
                        recv_frame.len()
                    );

                    match etherparse::PacketHeaders::from_ethernet_slice(&frame_ref) {
                        Err(value) => println!("Err {:?}", value),
                        Ok(value) => {
                            eprintln!("received frame in xdpscan rx loop:");
                            eprintln!("{:?}", value);
                        }
                    }
                }
                eprintln!(
                    "rx frames[0] = {}, rx frames[-1] = {}",
                    frames[0].addr(),
                    frames[frames.len() - 1].addr()
                );

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

                if total_frames_rcvd > 3 {
                    done = true;
                }
            }
        }
    }
}
