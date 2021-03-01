use futures::stream::TryStreamExt;
use rtnetlink::Handle;
use std::net::{IpAddr, Ipv4Addr};
use tokio::runtime::Runtime;

pub struct VethLink {
    handle: Handle,
    pub dev1_if_name: String,
    dev1_index: u32,
    pub dev2_if_name: String,
    dev2_index: u32,
}

#[derive(Clone, Debug)]
pub struct LinkIpAddr {
    addr: Ipv4Addr,
    prefix_len: u8,
}

impl LinkIpAddr {
    pub fn new(addr: Ipv4Addr, prefix_len: u8) -> Self {
        LinkIpAddr { addr, prefix_len }
    }

    pub fn octets(&self) -> [u8; 4] {
        self.addr.octets()
    }
}

#[derive(Clone, Debug)]
pub struct VethConfig {
    dev1_if_name: String,
    dev2_if_name: String,
    dev1_addr: [u8; 6],
    dev2_addr: [u8; 6],
    dev1_ip_addr: LinkIpAddr,
    dev2_ip_addr: LinkIpAddr,
}

impl VethConfig {
    pub fn new(
        dev1_if_name: String,
        dev2_if_name: String,
        dev1_addr: [u8; 6],
        dev2_addr: [u8; 6],
        dev1_ip_addr: LinkIpAddr,
        dev2_ip_addr: LinkIpAddr,
    ) -> Self {
        VethConfig {
            dev1_if_name,
            dev2_if_name,
            dev1_addr,
            dev2_addr,
            dev1_ip_addr,
            dev2_ip_addr,
        }
    }

    pub fn dev1_name(&self) -> &str {
        &self.dev1_if_name
    }

    pub fn dev2_name(&self) -> &str {
        &self.dev2_if_name
    }

    pub fn dev1_addr(&self) -> &[u8; 6] {
        &self.dev1_addr
    }

    pub fn dev2_addr(&self) -> &[u8; 6] {
        &self.dev2_addr
    }

    pub fn dev1_ip_addr(&self) -> &LinkIpAddr {
        &self.dev1_ip_addr
    }

    pub fn dev2_ip_addr(&self) -> &LinkIpAddr {
        &self.dev2_ip_addr
    }
}

async fn get_link_index(handle: &Handle, name: &str) -> anyhow::Result<u32> {
    Ok(handle
        .link()
        .get()
        .set_name_filter(name.into())
        .execute()
        .try_next()
        .await?
        .expect(format!("no link with name {} found", name).as_str())
        .header
        .index)
}

async fn set_link_up(handle: &Handle, index: u32) -> anyhow::Result<()> {
    Ok(handle.link().set(index).up().execute().await?)
}

async fn set_link_addr(handle: &Handle, index: u32, addr: Vec<u8>) -> anyhow::Result<()> {
    Ok(handle.link().set(index).address(addr).execute().await?)
}

async fn set_link_ip_addr(
    handle: &Handle,
    index: u32,
    link_ip_addr: &LinkIpAddr,
) -> anyhow::Result<()> {
    Ok(handle
        .address()
        .add(
            index,
            IpAddr::V4(link_ip_addr.addr.clone()),
            link_ip_addr.prefix_len,
        )
        .execute()
        .await?)
}

async fn delete_link(handle: &Handle, index: u32) -> anyhow::Result<()> {
    Ok(handle.link().del(index).execute().await?)
}

async fn build_veth_link(dev1_if_name: &str, dev2_if_name: &str) -> anyhow::Result<VethLink> {
    let (connection, handle, _) = rtnetlink::new_connection().unwrap();

    tokio::spawn(connection);

    handle
        .link()
        .add()
        .veth(dev1_if_name.into(), dev2_if_name.into())
        .execute()
        .await?;

    let dev1_index = get_link_index(&handle, dev1_if_name).await.expect(
        format!(
            "failed to retrieve index for dev1. Remove link manually: 'sudo ip link del {}'",
            dev1_if_name
        )
        .as_str(),
    );

    let dev2_index = get_link_index(&handle, dev2_if_name).await.expect(
        format!(
            "failed to retrieve index for dev2. Remove link manually: 'sudo ip link del {}'",
            dev1_if_name
        )
        .as_str(),
    );

    let dev1_if_name = dev1_if_name.into();
    let dev2_if_name = dev2_if_name.into();
    Ok(VethLink {
        handle,
        dev1_if_name,
        dev1_index,
        dev2_if_name,
        dev2_index,
    })
}

async fn configure_veth_link(veth_link: &VethLink, veth_config: &VethConfig) -> anyhow::Result<()> {
    set_link_up(&veth_link.handle, veth_link.dev1_index).await?;
    set_link_up(&veth_link.handle, veth_link.dev2_index).await?;

    set_link_addr(
        &veth_link.handle,
        veth_link.dev1_index,
        veth_config.dev1_addr.to_vec(),
    )
    .await?;

    set_link_addr(
        &veth_link.handle,
        veth_link.dev2_index,
        veth_config.dev2_addr.to_vec(),
    )
    .await?;

    set_link_ip_addr(
        &veth_link.handle,
        veth_link.dev1_index,
        &veth_config.dev1_ip_addr,
    )
    .await?;

    set_link_ip_addr(
        &veth_link.handle,
        veth_link.dev2_index,
        &veth_config.dev2_ip_addr,
    )
    .await?;

    Ok(())
}

pub fn setup_veth(conf: &VethConfig) -> (VethLink, Runtime) {
    let mut rt = tokio::runtime::Runtime::new().expect("failed to build tokio runtime");

    let veth_link = rt.block_on(
        async {
        build_veth_link(&conf.dev1_if_name, &conf.dev2_if_name).await.expect("failed to build veth link")
        }
    );

    rt.block_on(
        async {
            match configure_veth_link(&veth_link, &conf).await {
                Ok(()) => (),
                Err(e) => {
                    eprintln!("{}", e);
                    delete_link(&veth_link.handle, veth_link.dev1_index).await.unwrap_or_else(|_| {
                        let msg = format!("failed to delete link. May need to remove manually: 'sudo ip link del {}'", conf.dev1_if_name);
                        panic!(msg);
                    })
            }
        }
        });


    (veth_link, rt)
}

pub fn cleanup_veth(veth_link: &VethLink, rt: &mut Runtime) {
    rt.block_on(
        async {
                    delete_link(&veth_link.handle, veth_link.dev1_index).await.expect
                        ("failed to delete link. May need to remove manually: 'sudo ip link del '")
        });
}
