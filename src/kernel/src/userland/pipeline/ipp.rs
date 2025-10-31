use crate::{smp::MAX_CPU, sync::spin_mpsc::SpinMPSC};

const CHUNK_SIZE: usize = 64;
const PACKET_CAPACITY: usize = 512;

#[derive(Debug)]
struct PacketData {
    data: [u8; CHUNK_SIZE],
}

static PACKETS: [SpinMPSC<PacketData, PACKET_CAPACITY>; MAX_CPU] =
    [const { SpinMPSC::new() }; MAX_CPU];

pub struct IPPPipeline {}

impl IPPPipeline {}

pub trait IPPPacket {}
