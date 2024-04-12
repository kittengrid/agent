use boringtun::device::{DeviceConfig, DeviceHandle};
use lib::config::get_config;
use log::{debug, error, info};
use std::process::exit;

#[tokio::main]
async fn main() {
    let config = get_config();
    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", config.bind_address, config.bind_port))
            .await
            .unwrap();

    lib::publish_advertise_address(
        config.advertise_address.clone(),
        config.agent_token.clone(),
        config.api_url.clone(),
    )
    .await;
    lib::utils::initialize_logger();

    println!("GONA START TUNNEL");
    tokio::spawn(async {
        setup_tunnel().await;
    });
    println!("HOLA");
    lib::launch(listener).await;
}
// Makes a call to kittengrid API to register the agent advertise address
// so we can communicate with it
pub async fn fetch_network_config(address: String, token: String, api_url: String) {
    debug!("Publishing advertise address: {} to: {}", address, api_url);
    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}/api/agents/register", api_url))
        .json(&serde_json::json!({ "address": address }))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await;

    match res {
        Ok(res) => {
            if res.status().is_success() {
                debug!("Advertise address published successfully");
            } else {
                debug!("Failed to publish advertise address: {}", res.status());
            }
        }
        Err(e) => {
            debug!("Failed to publish advertise address: {}", e);
        }
    }
}

#[cfg(test)]
mod test_utils;

/*
#!/bin/bash
if [ ! -z $DEBUG ]; then
    set -x
    WG_TRACE="-v trace"
    CURL_TRACE="-v"
fi

cat << EOF > /tmp/registration.json
{
  vcs_provider: "${KG_VCS_PROVIDER}",
  vcs_id: "${KG_VCS_ID}",
  workflow_id: "${KG_WORKFLOW_ID}"
}
EOF

REGISTRATION_DATA=$(curl -XPOST "http://web:3000/api/agents/register" -H "Context-type: application/json" -H "Authorization: Bearer ${KG_API_KEY}" --data @/tmp/registration.json)
JWT_TOKEN=$(echo $REGISTRATION_DATA |jq .token -r)

curl -XPOST -H "Authorization: Bearer ${JWT_TOKEN}" "http://web:3000/api/peers" $CURL_TRACE | tee /tmp/registration_data.json
peers = ""
for i in 0 1; do
    WG_ADDRESS=$(cat /tmp/registration_data.json | jq .[$i].address -r)
    WG_PRIVATE_KEY=$(cat /tmp/registration_data.json | jq .[$i].private_key -r)
    WG_NETWORK=$(cat /tmp/registration_data.json | jq .[$i].network -r)
    WG_NETMASK=$(echo $WG_NETWORK|cut -d '/' -f2)
    WG_INTERFACE=wg$i
    WG_CONFIG_FILE=/tmp/wg$i.conf
    ENDPOINTS_FILE=/tmp/endpoints_data_$i.json

    # First we set up the header
    cat << EOF > $WG_CONFIG_FILE
[Interface]
PrivateKey = ${WG_PRIVATE_KEY}
ListenPort = 5182$i
EOF

    # Then the rest
    curl -XGET  -H "Authorization: Bearer ${JWT_TOKEN}" "http://web:3000/api/peers/endpoints?cidr=${WG_NETWORK}" $CURL_TRACE| tee $ENDPOINTS_FILE
    cat $ENDPOINTS_FILE | /json_to_config.rb >> $WG_CONFIG_FILE

    # Set up the VPN
    boringtun-cli -f $WG_INTERFACE --disable-drop-privileges -l /dev/stdout $WG_TRACE &
    sleep 1
    ip address add dev $WG_INTERFACE $WG_ADDRESS/$WG_NETMASK
    wg setconf $WG_INTERFACE $WG_CONFIG_FILE
    ip link set up dev $WG_INTERFACE
done

cat << EOF > service.json
{
  "peers": $(cat /tmp/registration_data.json  |jq '[.[] | .id ]'),
  "name": "agent-$(cat /tmp/registration_data.json  |jq '.[0].id' -r |cut -d - -f1)",
  "port": 8080,
  "path": "/sys/hello"
}
EOF

# Set up the service
curl  -H "Authorization: Bearer ${JWT_TOKEN}" --header "Content-Type: application/json" -XPOST "http://web:3000/api/peers/service" -d @service.json

# Start the agent

 */

async fn setup_tunnel() {
    let tun_name = "vg0";
    let n_threads = 4;

    let uapi_fd: i32 = -1;

    let config = DeviceConfig {
        n_threads,
        uapi_fd,
        use_connected_socket: true,
        use_multi_queue: true,
    };

    let mut device_handle: DeviceHandle = match DeviceHandle::new(tun_name, config) {
        Ok(d) => d,
        Err(e) => {
            // Notify parent that tunnel initialization failed
            error!("Failed to initialize tunnel {}", e);
            exit(1);
        }
    };

    info!("BoringTun started successfully");
    device_handle.wait();
}
