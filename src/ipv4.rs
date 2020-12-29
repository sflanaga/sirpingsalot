use anyhow::{anyhow,Context};
const MINIMUM_PACKET_SIZE: usize = 20;

type ResultS<T> = std::result::Result<T, anyhow::Error>;

#[derive(Debug, PartialEq)]
pub enum IpV4Protocol {
    Icmp,
}

impl IpV4Protocol {
    fn decode(data: u8) -> Option<Self> {
        match data {
            1 => Some(IpV4Protocol::Icmp),
            _ => None,
        }
    }
}

pub struct IpV4Packet<'a> {
    pub protocol: IpV4Protocol,
    pub ttl: u8,
    pub data: &'a [u8],
}

impl<'a> IpV4Packet<'a> {
    pub fn decode(data: &'a [u8]) -> ResultS<Self> {
        if data.len() < MINIMUM_PACKET_SIZE {
            //return Err(Error::TooSmallHeader);
            return Err(anyhow!("header too small"))
        }
        let byte0 = data[0];
        let version = (byte0 & 0xf0) >> 4;
        let header_size = 4 * ((byte0 & 0x0f) as usize);
        let ttl = data[8];
        if version != 4 {
//            return Err(Error::InvalidVersion);
            return Err(anyhow!("invalid version of {} expected {}", version, 4))
        }

        if data.len() < header_size {
            return Err(anyhow!("data size less than header size: {} < {}", data.len(), header_size))
        }

        let protocol = match IpV4Protocol::decode(data[9]) {
            Some(protocol) => protocol,
            None => return Err(anyhow!("unknown protocol encoded {:x}", data[9]))

        };

        Ok(Self {
            protocol: protocol,
            ttl,
            data: &data[header_size..],
        })
    }
}
