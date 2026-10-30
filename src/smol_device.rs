//! In-memory IP device: no host interfaces, routes, or privileged system calls.
use std::collections::VecDeque;

use smoltcp::{
    phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
    time::Instant,
};

pub(crate) const MTU: u16 = 1360;

#[derive(Default)]
pub(crate) struct SmolIpDevice {
    pub incoming: VecDeque<Vec<u8>>,
    pub outgoing: VecDeque<Vec<u8>>,
}

pub(crate) struct SmolRxToken {
    bytes: Vec<u8>,
}

pub(crate) struct SmolTxToken<'a> {
    queue: &'a mut VecDeque<Vec<u8>>,
}

impl RxToken for SmolRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.bytes)
    }
}

impl TxToken for SmolTxToken<'_> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut bytes = vec![0u8; len];
        let result = f(&mut bytes);
        self.queue.push_back(bytes);
        result
    }
}

impl Device for SmolIpDevice {
    type RxToken<'a> = SmolRxToken;
    type TxToken<'a> = SmolTxToken<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let bytes = self.incoming.pop_front()?;
        Some((
            SmolRxToken { bytes },
            SmolTxToken {
                queue: &mut self.outgoing,
            },
        ))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(SmolTxToken {
            queue: &mut self.outgoing,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = usize::from(MTU);
        caps
    }
}
