use bitcoincore_rpc::{Auth, Client as BitcoinClient, RpcApi};
use reqwest::blocking::Client;
use serde_json::Value;

/// Call Alice's Lightning node via CLN REST API on port 3010
fn call_alice_ln(method: &str, params: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let rune = std::env::var("ALICE_RUNE")?;
    let url = format!("http://localhost:3010/v1/{}", method);

    let client = Client::new();
    let response = client
        .post(&url)
        .json(&params)
        .header("Rune", rune)
        .send()?
        .json::<Value>()?;

    Ok(response)
}

/// Call Bob's Lightning node via CLN REST API on port 3011
fn call_bob_ln(method: &str, params: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let rune = std::env::var("BOB_RUNE")?;
    let url = format!("http://localhost:3011/v1/{}", method);

    let client = Client::new();
    let response = client
        .post(&url)
        .json(&params)
        .header("Rune", rune)
        .send()?
        .json::<Value>()?;

    Ok(response)
}

/// Call Carols's Lightning node via CLN REST API on port 3012
fn call_carol_ln(method: &str, params: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let rune = std::env::var("CAROL_RUNE")?;
    let url = format!("http://localhost:3012/v1/{}", method);

    let client = Client::new();
    let response = client
        .post(&url)
        .json(&params)
        .header("Rune", rune)
        .send()?
        .json::<Value>()?;

    Ok(response)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Bitcoin RPC client
    let bitcoin_rpc = BitcoinClient::new(
        "http://localhost:18443",
        Auth::UserPass("alice".to_string(), "password".to_string()),
    )?;

    println!("Blockchain Info: {:?}", bitcoin_rpc.get_blockchain_info()?);
    // Get Alice's node info
    let alice_info = call_alice_ln("getinfo", Value::Null)?;
    println!("Alice Node Info: {:?}", alice_info);

    // Get Bob's node info
    let bob_info = call_bob_ln("getinfo", Value::Null)?;
    println!("Bob Node Info: {:?}", bob_info);

    // Get Carol's node info
    let carol_info = call_carol_ln("getinfo", Value::Null)?;
    println!("Carol Node Info: {:?}", carol_info);

    let run = |program: &str, args: &[&str]| -> Result<String, Box<dyn std::error::Error>> {
        let output = std::process::Command::new(program).args(args).output()?;

        if !output.status.success() {
            return Err(format!(
                "{} {:?} failed\nstdout: {}\nstderr: {}",
                program,
                args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    };

    let bitcoin_cli = |args: &[&str]| -> Result<String, Box<dyn std::error::Error>> {
        let mut command = vec![
            "exec",
            "bitcoind",
            "bitcoin-cli",
            "-regtest",
            "-rpcuser=alice",
            "-rpcpassword=password",
        ];
        command.extend_from_slice(args);
        run("docker", &command)
    };

    let bitcoin_cli_json = |args: &[&str]| -> Result<Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(&bitcoin_cli(args)?)?)
    };

    let mine_blocks = |blocks: u64| -> Result<(), Box<dyn std::error::Error>> {
        let address = bitcoin_cli(&["-rpcwallet=mining_wallet", "getnewaddress"])?;
        bitcoin_cli(&[
            "-rpcwallet=mining_wallet",
            "generatetoaddress",
            &blocks.to_string(),
            &address,
        ])?;
        Ok(())
    };

    let wait_for_channel = |node_name: &str,
                            peer_id: &str|
     -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..90 {
            let channels = match node_name {
                "alice" => call_alice_ln("listpeerchannels", serde_json::json!({}))?,
                "bob" => call_bob_ln("listpeerchannels", serde_json::json!({}))?,
                _ => return Err("unknown node".into()),
            };

            let normal = channels
                .get("channels")
                .and_then(Value::as_array)
                .ok_or("listpeerchannels response missing channels")?
                .iter()
                .any(|channel| {
                    channel.get("peer_id").and_then(Value::as_str) == Some(peer_id)
                        && channel.get("state").and_then(Value::as_str) == Some("CHANNELD_NORMAL")
                });

            if normal {
                return Ok(());
            }

            std::thread::sleep(std::time::Duration::from_secs(2));
        }

        Err(format!(
            "channel with peer {} did not reach CHANNELD_NORMAL",
            peer_id
        )
        .into())
    };

    let wait_for_forward = || -> Result<Value, Box<dyn std::error::Error>> {
        for _ in 0..60 {
            let forwards = call_bob_ln("listforwards", serde_json::json!({}))?;
            if let Some(forward) =
                forwards
                    .get("forwards")
                    .and_then(Value::as_array)
                    .and_then(|items| {
                        items.iter().find(|forward| {
                            forward.get("status").and_then(Value::as_str) == Some("settled")
                        })
                    })
            {
                return Ok(forward.clone());
            }

            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        Err("Bob did not report a settled forward for the payment".into())
    };

    let wait_for_node_sync = |target_height: u64| -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..90 {
            let alice_height = call_alice_ln("getinfo", Value::Null)?
                .get("blockheight")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let bob_height = call_bob_ln("getinfo", Value::Null)?
                .get("blockheight")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let carol_height = call_carol_ln("getinfo", Value::Null)?
                .get("blockheight")
                .and_then(Value::as_u64)
                .unwrap_or(0);

            if alice_height >= target_height
                && bob_height >= target_height
                && carol_height >= target_height
            {
                return Ok(());
            }

            std::thread::sleep(std::time::Duration::from_secs(2));
        }

        Err("Lightning nodes did not sync to bitcoind".into())
    };

    let wait_for_confirmed_output = |node_name: &str| -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..90 {
            let funds = match node_name {
                "alice" => call_alice_ln("listfunds", serde_json::json!({}))?,
                "bob" => call_bob_ln("listfunds", serde_json::json!({}))?,
                _ => return Err("unknown node".into()),
            };

            let has_confirmed_output = funds
                .get("outputs")
                .and_then(Value::as_array)
                .map(|outputs| {
                    outputs.iter().any(|output| {
                        output.get("status").and_then(Value::as_str) == Some("confirmed")
                            && output.get("reserved").and_then(Value::as_bool) == Some(false)
                    })
                })
                .unwrap_or(false);

            if has_confirmed_output {
                return Ok(());
            }

            std::thread::sleep(std::time::Duration::from_secs(2));
        }

        Err(format!("{} did not see confirmed funding output", node_name).into())
    };

    let wait_for_route = |payee_id: &str| -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..90 {
            if run(
                "docker",
                &[
                    "exec",
                    "alice",
                    "lightning-cli",
                    "--network=regtest",
                    "getroute",
                    payee_id,
                    "100000000msat",
                    "1",
                ],
            )
            .is_ok()
            {
                return Ok(());
            }

            std::thread::sleep(std::time::Duration::from_secs(2));
        }

        Err(format!("Alice did not learn a route to {}", payee_id).into())
    };

    // Create a bitcoin wallet named 'mining_wallet' if it doesn't exist
    let wallets = bitcoin_cli_json(&["listwallets"])?;
    let wallet_loaded = wallets
        .as_array()
        .ok_or("listwallets did not return an array")?
        .iter()
        .any(|wallet| wallet.as_str() == Some("mining_wallet"));

    if !wallet_loaded {
        if bitcoin_cli(&["loadwallet", "mining_wallet"]).is_err() {
            bitcoin_cli(&["createwallet", "mining_wallet"])?;
        }
    }

    // Generate a mining address and mine initial blocks
    mine_blocks(101)?;
    let chain_height = bitcoin_cli(&["getblockcount"])?.parse::<u64>()?;
    wait_for_node_sync(chain_height)?;

    // Create and fund an on-chain address for Alice
    let alice_address = call_alice_ln("newaddr", serde_json::json!({}))?
        .get("bech32")
        .and_then(Value::as_str)
        .ok_or("Alice newaddr response missing bech32")?
        .to_string();
    bitcoin_cli(&[
        "-rpcwallet=mining_wallet",
        "sendtoaddress",
        &alice_address,
        "0.01000000",
    ])?;

    // Create and fund an on-chain address for Bob
    let bob_address = call_bob_ln("newaddr", serde_json::json!({}))?
        .get("bech32")
        .and_then(Value::as_str)
        .ok_or("Bob newaddr response missing bech32")?
        .to_string();
    bitcoin_cli(&[
        "-rpcwallet=mining_wallet",
        "sendtoaddress",
        &bob_address,
        "0.01000000",
    ])?;

    // Mine blocks to confirm funding transactions
    mine_blocks(6)?;
    let chain_height = bitcoin_cli(&["getblockcount"])?.parse::<u64>()?;
    wait_for_node_sync(chain_height)?;
    wait_for_confirmed_output("alice")?;
    wait_for_confirmed_output("bob")?;

    // Verify Alice's on-chain balance
    println!(
        "Alice funds: {:?}",
        call_alice_ln("listfunds", serde_json::json!({}))?
    );

    // Verify Bob's on-chain balance
    println!(
        "Bob funds: {:?}",
        call_bob_ln("listfunds", serde_json::json!({}))?
    );

    // Get node IDs for Alice, Bob, and Carol
    let alice_id = alice_info
        .get("id")
        .and_then(Value::as_str)
        .ok_or("Alice getinfo response missing id")?
        .to_string();
    let bob_id = bob_info
        .get("id")
        .and_then(Value::as_str)
        .ok_or("Bob getinfo response missing id")?
        .to_string();
    let carol_id = carol_info
        .get("id")
        .and_then(Value::as_str)
        .ok_or("Carol getinfo response missing id")?
        .to_string();

    // Connect them as peers
    let _ = call_alice_ln(
        "connect",
        serde_json::json!({
            "id": bob_id,
            "host": "bob",
            "port": 9735
        }),
    );
    let _ = call_bob_ln(
        "connect",
        serde_json::json!({
            "id": carol_id,
            "host": "carol",
            "port": 9735
        }),
    );

    // Alice opens a 500,000 sat channel with Bob
    call_alice_ln(
        "fundchannel",
        serde_json::json!({
            "id": bob_id,
            "amount": "500000sat"
        }),
    )?;

    // Bob opens a 300,000 sat channel with Carol
    call_bob_ln(
        "fundchannel",
        serde_json::json!({
            "id": carol_id,
            "amount": "300000sat"
        }),
    )?;

    // Mine at least 6 blocks to confirm channels
    mine_blocks(6)?;
    let chain_height = bitcoin_cli(&["getblockcount"])?.parse::<u64>()?;
    wait_for_node_sync(chain_height)?;

    // Wait for channels to reach CHANNELD_NORMAL state
    wait_for_channel("alice", &bob_id)?;
    wait_for_channel("bob", &carol_id)?;
    wait_for_route(&carol_id)?;

    // Carol generates a 100,000 sat invoice with label "multihop_<timestamp>" and description "Multi-Hop Payment"
    let label = format!(
        "multihop_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
    );
    let invoice = call_carol_ln(
        "invoice",
        serde_json::json!({
            "amount_msat": "100000000msat",
            "label": label,
            "description": "Multi-Hop Payment"
        }),
    )?;

    // Extract the BOLT11 string and payment hash from the invoice
    let bolt11 = invoice
        .get("bolt11")
        .and_then(Value::as_str)
        .ok_or("invoice response missing bolt11")?
        .to_string();
    let payment_hash = invoice
        .get("payment_hash")
        .and_then(Value::as_str)
        .ok_or("invoice response missing payment_hash")?
        .to_string();

    // Alice pays Carol's BOLT11 invoice (routed through Bob)
    let pay_output = run(
        "docker",
        &[
            "exec",
            "alice",
            "lightning-cli",
            "--network=regtest",
            "pay",
            &bolt11,
        ],
    )?;
    let pay: Value = serde_json::from_str(&pay_output)?;

    // Extract payment preimage and status
    let preimage = if let Some(preimage) = pay
        .get("payment_preimage")
        .or_else(|| pay.get("preimage"))
        .and_then(Value::as_str)
    {
        preimage.to_string()
    } else {
        let pays = call_alice_ln("listpays", serde_json::json!({}))?;
        pays.get("pays")
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("payment_hash").and_then(Value::as_str) == Some(&payment_hash)
                        && item.get("status").and_then(Value::as_str) == Some("complete")
                })
            })
            .and_then(|item| {
                item.get("payment_preimage")
                    .or_else(|| item.get("preimage"))
                    .and_then(Value::as_str)
            })
            .ok_or("listpays response missing payment preimage")?
            .to_string()
    };

    // Verify Alice's balance decreased
    println!(
        "Alice funds after payment: {:?}",
        call_alice_ln("listfunds", serde_json::json!({}))?
    );

    // Verify Carol's balance increased
    println!(
        "Carol invoices: {:?}",
        call_carol_ln("listinvoices", serde_json::json!({}))?
    );

    // Verify Bob's balance. Is there any difference? Why is it?
    println!(
        "Bob funds after forwarding: {:?}",
        call_bob_ln("listfunds", serde_json::json!({}))?
    );

    // Verify Bob forwarded the payment using listforwards and extract payment_hash from it
    let forward = wait_for_forward()?;
    let fee_msat = forward
        .get("fee_msat")
        .and_then(|fee| {
            fee.as_str()
                .map(|s| s.trim_end_matches("msat").to_string())
                .or_else(|| fee.as_i64().map(|n| n.to_string()))
        })
        .ok_or("forward response missing fee_msat")?;
    let forwarded_hash = forward
        .get("payment_hash")
        .and_then(Value::as_str)
        .unwrap_or(&payment_hash);

    // Write to out.txt:
    // Payment Hash
    // Payment Preimage
    // BOLT11 Invoice
    // Payer_ID
    // Payee_ID
    // Fee_msat
    // Payment_Hash from Bob's forwarded payment
    std::fs::write(
        "../out.txt",
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
            payment_hash, preimage, bolt11, alice_id, carol_id, fee_msat, forwarded_hash
        ),
    )?;

    Ok(())
}
