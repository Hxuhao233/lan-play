use std::sync::Arc;
use crate::get_addr::{get_mac};
use smoltcp::phy::{DeviceCapabilities,RxToken,TxToken};
use rawsock::traits::{DynamicInterface, Library};
use rawsock::InterfaceDescription;
use crossbeam_utils::{thread, sync::Parker};
use smoltcp::{
    iface::{EthernetInterfaceBuilder, NeighborCache, EthernetInterface},
    wire::{IpCidr, EthernetAddress},
    socket::{SocketSet},
    time::{Instant},
};
use std::collections::BTreeMap;
use crate::duplex::{ChannelPort, Sender};
use log::{warn, debug};
use super::{Error, ErrorWithDesc};

type Packet = Vec<u8>;

pub struct RawsockInterfaceSet {
    lib: Box<dyn Library>,
    all_interf: Vec<rawsock::InterfaceDescription>,
    ip: smoltcp::wire::IpCidr,
}

pub struct RawsockDevice {
    pub port: ChannelPort<Packet>,
}

pub struct RawsockRunner<'a> {
    pub port: ChannelPort<Packet>,
    pub interface: Arc<dyn DynamicInterface<'a> + 'a>,
}

pub struct RawsockInterface<'a> {
    pub desc: InterfaceDescription,
    mac: EthernetAddress,
    data_link: rawsock::DataLink,
    device: RawsockDevice,
    port: ChannelPort<Packet>,
    interface: Arc<dyn DynamicInterface<'a> + 'a>,
    ip: smoltcp::wire::IpCidr,
    // dummy: &'a (),
}

impl RawsockInterfaceSet {
    pub fn new(lib: Box<dyn Library>, ip: IpCidr) -> Result<RawsockInterfaceSet, rawsock::Error> {
        let all_interf = lib.all_interfaces()?;
        Ok(RawsockInterfaceSet {
            lib,
            all_interf,
            ip,
        })
    }
    pub fn lib_version(&self) -> rawsock::LibraryVersion {
        self.lib.version()
    }
    pub fn open_all_interface(&self) -> (Vec<RawsockInterface>, Vec<ErrorWithDesc>) {
        let all_interf = self.all_interf.clone();
        let (opened, errored): (Vec<_>, _) = all_interf
            .into_iter()
            .map(|i| self.create_device(i))
            .partition(Result::is_ok);
        (
            opened.into_iter().map(Result::unwrap).collect::<Vec<_>>(),
            errored.into_iter().map(|i| i.err().unwrap()).collect::<Vec<_>>()
        )
    }
    fn create_device<'a>(&'a self, desc: InterfaceDescription) -> Result<RawsockInterface<'a>, ErrorWithDesc> {
        self.create_device_inner(&desc).map_err(|err| { ErrorWithDesc(err, desc) })
    }
    fn create_device_inner<'a>(&'a self, desc: &InterfaceDescription) -> Result<RawsockInterface<'a>, Error> {
        let name = &desc.name;
        let mut interface = self.lib.open_interface_arc(name)?;
        Arc::get_mut(&mut interface).ok_or(Error::Other("Bad Arc"))?.set_filter("icmp")?;

        let data_link = interface.data_link();
        if let rawsock::DataLink::Ethernet = data_link {} else {
            return Err(Error::WrongDataLink(data_link));
        }

        let (port1, port2) = ChannelPort::new();

        let mac = get_mac(name)?;
        Ok(RawsockInterface {
            data_link,
            desc: desc.clone(),
            port: port1,
            device: RawsockDevice {
                port: port2
            },
            mac,
            interface,
            ip: self.ip.clone()
        })
    }
}

impl<'a> RawsockInterface<'a> {
    pub fn name(&self) -> &String {
        &self.desc.name
    }
    pub fn mac(&self) -> &EthernetAddress {
        &self.mac
    }
    pub fn data_link(&self) -> rawsock::DataLink {
        self.data_link
    }
    pub fn split_device(self) -> (RawsockDevice, RawsockRunner<'a>) {
        (self.device, RawsockRunner {
            port: self.port,
            interface: self.interface
        })
    }
    pub fn split_iface<'b, 'c, 'e>(self) -> (
            EthernetInterface<'b, 'c, 'e, RawsockDevice>,
            RawsockRunner<'a>
    ) {
        let ethernet_addr = self.mac().clone();
        let device = self.device;
        let interface = self.interface;
        let neighbor_cache = NeighborCache::new(BTreeMap::new());
        let ip_addrs = [
            self.ip,
        ];
        let iface = EthernetInterfaceBuilder::new(device)
                .ethernet_addr(ethernet_addr)
                .neighbor_cache(neighbor_cache)
                .ip_addrs(ip_addrs)
                .finalize();
        (iface, RawsockRunner {
            port: self.port,
            interface
        })
    }
}

pub struct RawRxToken(Packet);

impl RxToken for RawRxToken {
    fn consume<R, F>(self, _timestamp: Instant, f: F) -> smoltcp::Result<R>
        where F: (FnOnce(&[u8]) -> smoltcp::Result<R>)
    {
        let p = &self.0;
        let result = f(p);
        result
    }
}


pub struct RawTxToken(Sender::<Packet>);

impl<'a> TxToken for RawTxToken {
    fn consume<R, F>(self, _timestamp: Instant, len: usize, f: F) -> smoltcp::Result<R>
        where F: FnOnce(&mut [u8]) -> smoltcp::Result<R>
    {
        let mut buffer = Vec::new();
        buffer.resize(len, 0);
        let result = f(&mut buffer);
        let sender = self.0;
        let sent = sender.send(buffer);
        if !sent.is_ok() {
            println!("send failed {}", len);
        }
        result
    }
}

impl<'d> smoltcp::phy::Device<'d> for RawsockDevice {
    type RxToken = RawRxToken;
    type TxToken = RawTxToken;

    fn receive(&'d mut self) -> Option<(Self::RxToken, Self::TxToken)> {
        self.port.try_recv().ok().map(|packet| {(
            RawRxToken(packet),
            RawTxToken(self.port.clone_sender())
        )})
    }
    fn transmit(&'d mut self) -> Option<Self::TxToken> {
        Some(RawTxToken(self.port.clone_sender()))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1536;
        caps.max_burst_size = Some(1);
        caps
    }
}
