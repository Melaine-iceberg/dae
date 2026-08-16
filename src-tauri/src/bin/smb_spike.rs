use std::time::Instant;

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let addr = std::env::var("SMB_ADDR").unwrap_or_else(|_| "127.0.0.1:445".into());

        println!("== guest connect to {addr} ==");
        match smb2::connect(&addr, "", "").await {
            Ok(mut client) => {
                println!("guest connect OK");
                match client.list_shares().await {
                    Ok(shares) => {
                        println!("shares: {:?}", shares.iter().map(|s| s.name.clone()).collect::<Vec<_>>());
                        run_roundtrip(&mut client).await;
                    }
                    Err(e) => println!("guest list_shares failed: {e}"),
                }
            }
            Err(e) => println!("guest connect failed: {e}"),
        }

        if let (Ok(user), Ok(pass)) = (std::env::var("SMB_USER"), std::env::var("SMB_PASS")) {
            println!("\n== authenticated connect as {user} ==");
            match smb2::connect(&addr, &user, &pass).await {
                Ok(mut client) => {
                    println!("auth connect OK");
                    run_roundtrip(&mut client).await;
                }
                Err(e) => println!("auth connect failed: {e}"),
            }
        }
    });
}

async fn run_roundtrip(client: &mut smb2::SmbClient) {
    let share = std::env::var("SMB_SHARE").unwrap_or_else(|_| "DevelopmentFiles".into());
    let mut tree = match client.connect_share(&share).await {
        Ok(tree) => tree,
        Err(e) => return println!("connect_share({share}) failed: {e}"),
    };

    let started = Instant::now();
    let entries = match client.list_directory(&mut tree, "").await {
        Ok(entries) => entries,
        Err(e) => return println!("list_directory failed: {e}"),
    };
    println!(
        "list_directory: {} entries in {:?}; first 5: {:?}",
        entries.len(),
        started.elapsed(),
        entries
            .iter()
            .take(5)
            .map(|e| format!("{}{}({}b)", e.name, if e.is_directory { "/" } else { "" }, e.size))
            .collect::<Vec<_>>()
    );

    let path = std::env::var("SMB_PATH").unwrap_or_else(|_| "dae-spike.txt".into());

    let n = client
        .write_file(&mut tree, &path, b"hello from dae vfs")
        .await
        .expect("write_file");
    println!("write_file: {n} bytes");

    let data = client.read_file(&mut tree, &path).await.expect("read_file");
    assert_eq!(data, b"hello from dae vfs");
    println!("read_file roundtrip OK");

    client.stat(&mut tree, &path).await.expect("stat");
    client.rename(&mut tree, &path, "dae-spike-renamed.txt").await.expect("rename");
    client.delete_file(&mut tree, "dae-spike-renamed.txt").await.expect("delete_file");
    client.create_directory(&mut tree, "dae-spike-dir").await.expect("create_directory");
    client.delete_directory(&mut tree, "dae-spike-dir").await.expect("delete_directory");
    println!("FULL ROUNDTRIP OK");
}
