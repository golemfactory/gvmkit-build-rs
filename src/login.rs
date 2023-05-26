use crate::upload::check_login;
use keyring::{Entry, Error};
use std::io;

const APPLICATION_NAME: &str = "gvmkit-build-rs";
pub async fn login(user_name: Option<&str>, force: bool) -> anyhow::Result<(String, String)> {
    let user_name = if let Some(user_name) = user_name {
        user_name.to_string()
    } else {
        println!("Provide username to registry portal:");
        let mut user_name = String::new();
        io::stdin().read_line(&mut user_name)?;
        user_name.trim().to_string()
    };

    if !force {
        if let Some(pat) = get_credentials(&user_name).await? {
            println!("Credentials already found {}", user_name);
            let logged_in = check_login(&user_name, &pat).await?;
            if logged_in {
                return Ok((user_name, pat));
            }
        }
    };
    println!("Provide access token (go to settings in registry portal to obtain one):");
    let pat = rpassword::read_password().unwrap();
    let pat = pat.trim().to_string();
    let can_log_in = check_login(&user_name, &pat).await?;
    if can_log_in {
        save_credentials(&user_name, &pat).await?;
    } else {
        return Err(anyhow::anyhow!("Login failed"));
    }
    Ok((user_name, pat))
}

pub async fn save_credentials(user_name: &str, password: &str) -> anyhow::Result<()> {
    if let Some(_pass) = get_credentials(user_name).await? {
        let entry = Entry::new(APPLICATION_NAME, user_name)?;
        entry.set_password(password)?;
        println!(
            "Credentials saved - entry overwritten for: {} {}",
            APPLICATION_NAME, user_name
        );
    } else {
        let entry = Entry::new(APPLICATION_NAME, user_name)?;
        entry.set_password(password)?;
        println!(
            "Credentials saved - entry created for: {} {}",
            APPLICATION_NAME, user_name
        );
    }
    Ok(())
}

pub async fn get_credentials(user_name: &str) -> anyhow::Result<Option<String>> {
    let entry = Entry::new(APPLICATION_NAME, user_name)?;
    match entry.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(e) => match e {
            Error::NoEntry => Ok(None),
            _ => Err(e.into()),
        },
    }
}
