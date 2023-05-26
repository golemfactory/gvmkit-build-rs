use crate::upload::check_login;
use keyring::{Entry, Error};
use std::io;
use std::io::{stdout, Write};

const CREDENTIAL_SERVICE_FIELD: &str = "gvmkit-build-rs";
const CREDENTIAL_USER_FIELD: &str = "default";


pub async fn check_if_valid_login() -> anyhow::Result<bool> {
    if let Some((user_name, pat)) = get_credentials().await? {
        println!(" -- credentials found {}", user_name);
        let logged_in = check_login(&user_name, &pat).await?;
        if logged_in {
            return Ok(true);
        }
    } else {
        println!(" -- credentials not found");
    }
    Ok(false)
}

pub async fn login(user_name: Option<&str>, force: bool) -> anyhow::Result<(String, String)> {
    if !force {
        if let Some((user_name, pat)) = get_credentials().await? {
            println!(" -- credentials already found {}", user_name);
            let logged_in = check_login(&user_name, &pat).await?;
            if logged_in {
                return Ok((user_name, pat));
            }
        }
    };
    let user_name = if let Some(user_name) = user_name {
        print!(" -- provide username [{user_name}]:");
        stdout().flush()?;
        let mut input_data = String::new();
        io::stdin().read_line(&mut input_data)?;
        let input_data = input_data.trim().to_string();
        if input_data.is_empty() {
            user_name.to_string()
        } else {
            input_data
        }
    } else {
        print!(" -- provide username:");
        stdout().flush()?;
        let mut user_name = String::new();
        io::stdin().read_line(&mut user_name)?;
        user_name.trim().to_string()
    };


    println!(" -- provide access token:");
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
    let entry = Entry::new(CREDENTIAL_SERVICE_FIELD, CREDENTIAL_USER_FIELD)?;
    entry.set_password(&format!("{}:{}", user_name, password))?;
    println!(
        "Credentials saved - entry created for: {} {}",
        CREDENTIAL_SERVICE_FIELD, CREDENTIAL_USER_FIELD
    );
    Ok(())
}

pub async fn remove_credentials() -> anyhow::Result<()> {
    let entry = Entry::new(CREDENTIAL_SERVICE_FIELD, CREDENTIAL_USER_FIELD)?;
    match entry.delete_password() {
        Ok(_) => {
            println!(
                "Credentials removed - entry removed for: {} {}",
                CREDENTIAL_SERVICE_FIELD, CREDENTIAL_USER_FIELD
            );
            Ok(())
        },
        Err(e) => match e {
            Error::NoEntry => {
                println!(
                    "Credentials not found - entry not found for: {} {}",
                    CREDENTIAL_SERVICE_FIELD, CREDENTIAL_USER_FIELD
                );
                Ok(())
            },
            _ => Err(e.into()),
        },
    }
}

pub async fn get_credentials() -> anyhow::Result<Option<(String, String)>> {
    let entry = Entry::new(CREDENTIAL_SERVICE_FIELD, CREDENTIAL_USER_FIELD)?;
    match entry.get_password() {
        Ok(password) => {
            let mut split = password.split(':');
            let user = split.next();
            let pat = split.next();
            if let (Some(user), Some(pat)) = (user, pat) {
                Ok(Some((user.to_string(), pat.to_string())))
            } else {
                Ok(None)
            }
        },
        Err(e) => match e {
            Error::NoEntry => Ok(None),
            _ => Err(e.into()),
        },
    }
}
