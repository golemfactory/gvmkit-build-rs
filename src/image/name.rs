pub struct ImageName {
    pub user: Option<String>,
    pub repository: String,
    pub tag: String,
}

impl ImageName {
    pub fn to_base_name(&self) -> String {
        let mut res = String::new();
        if let Some(user) = &self.user {
            res.push_str(user);
            res.push('/');
        }
        res.push_str(&self.repository);
        res
    }

    pub fn to_normalized_name(&self) -> String {
        let mut res = String::new();
        if let Some(user) = &self.user {
            res.push_str(user);
            res.push('/');
        }
        res.push_str(&self.repository);
        res.push(':');
        res.push_str(&self.tag);
        res
    }

    pub fn from_str_name(image_name: &str) -> anyhow::Result<Self> {
        let mut contains_alpha = false;
        for (pos, c) in image_name.chars().enumerate() {
            if c.is_whitespace() {
                return Err(anyhow::anyhow!(
                    "Invalid image name: {}. Cannot contain whitespaces",
                    image_name
                ));
            }
            if c.is_ascii_alphanumeric() {
                contains_alpha = true;
            } else if c == '-' || c == '_' || c == '.' || c == '/' || c == ':' {
                // ok
            } else {
                return Err(anyhow::anyhow!("Invalid image name: {}. Contains at least one invalid character: '{}' at pos {}", image_name, c, pos));
            }
        }
        if !contains_alpha {
            return Err(anyhow::anyhow!(
                "Invalid image name: {}. Must contain alphanumeric characters",
                image_name
            ));
        }

        if image_name.starts_with(':') {
            return Err(anyhow::anyhow!(
                "Invalid image name: {}. Cannot start with ':'",
                image_name
            ));
        }
        if image_name.starts_with('/') {
            return Err(anyhow::anyhow!(
                "Invalid image name: {}. Cannot start with '/'",
                image_name
            ));
        }
        if image_name.starts_with('/') {
            return Err(anyhow::anyhow!(
                "Invalid image name: {}. Cannot start with '/'",
                image_name
            ));
        }
        if image_name.matches(':').count() > 1 {
            return Err(anyhow::anyhow!(
                "Invalid image name: {}. ':' can occur only once",
                image_name
            ));
        }
        if image_name.matches('/').count() > 1 {
            return Err(anyhow::anyhow!(
                "Invalid image name: {}. '/' can occur only once",
                image_name
            ));
        }
        let (base_part, tag_part) = if image_name.contains(':') {
            let mut split = image_name.split(':');
            (
                split.next().expect("Split has to be here"),
                split.next().expect("Split has to be here"),
            )
        } else {
            (image_name, "latest")
        };
        if tag_part.is_empty() {
            return Err(anyhow::anyhow!(
                "Invalid image name: {}. Tag part cannot be empty",
                image_name
            ));
        }
        let (user, repo) = if base_part.contains('/') {
            let mut split = base_part.split('/');
            (
                Some(split.next().expect("Split has to be here")),
                split.next().expect("Split has to be here"),
            )
        } else {
            (None, base_part)
        };

        Ok(Self {
            user: user.map(|s| s.to_string()),
            repository: repo.to_string(),
            tag: tag_part.to_string(),
        })
    }
}

#[test]
fn test_descriptor_creation() {
    {
        let image_name = ImageName::from_str_name("test").unwrap();
        assert_eq!(image_name.to_base_name(), "test");
        assert_eq!(image_name.repository, "test");
        assert_eq!(image_name.user, None);
        assert_eq!(image_name.to_normalized_name(), "test:latest");
    }
    {
        let image_name = ImageName::from_str_name("user-test/repo_te.st").unwrap();
        assert_eq!(image_name.to_base_name(), "user-test/repo_te.st");
        assert_eq!(image_name.repository, "repo_te.st");
        assert_eq!(image_name.user, Some("user-test".to_string()));
        assert_eq!(
            image_name.to_normalized_name(),
            "user-test/repo_te.st:latest"
        );
    }
    {
        let image_name = ImageName::from_str_name("user/repo:version-1.3.5").unwrap();
        assert_eq!(image_name.to_base_name(), "user/repo");
        assert_eq!(image_name.repository, "repo");
        assert_eq!(image_name.tag, "version-1.3.5");
        assert_eq!(image_name.user, Some("user".to_string()));
        assert_eq!(image_name.to_normalized_name(), "user/repo:version-1.3.5");
    }
    {
        let image_name = ImageName::from_str_name("repo:version-1.3.5").unwrap();
        assert_eq!(image_name.to_base_name(), "repo");
        assert_eq!(image_name.repository, "repo");
        assert_eq!(image_name.tag, "version-1.3.5");
        assert_eq!(image_name.user, None);
        assert_eq!(image_name.to_normalized_name(), "repo:version-1.3.5");
    }

    {
        //test invalid names here
        assert!(ImageName::from_str_name("^").is_err());
        assert!(ImageName::from_str_name("..").is_err());
        assert!(ImageName::from_str_name(":tag").is_err());
        assert!(ImageName::from_str_name("repo:").is_err());
        assert!(ImageName::from_str_name("repo:version,2").is_err());
        assert!(ImageName::from_str_name("repo:repo1:repo2").is_err());
        assert!(ImageName::from_str_name("user/repo/sub_repo").is_err());
        assert!(ImageName::from_str_name("user/rep ").is_err());
        assert!(ImageName::from_str_name("use r/rep").is_err());
        assert!(ImageName::from_str_name("usß/rep").is_err());
    }
}
