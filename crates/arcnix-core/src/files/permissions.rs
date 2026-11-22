//! POSIX permission values; ownership and file type bits are never written.
use anyhow::{Result, ensure};

#[derive(Clone, Debug)]
pub struct Metadata {
    pub hidden_attribute: bool,
    pub size: u64,
    pub modified: Option<u64>,
    pub directory: bool,
    pub symlink: bool,
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub owner_name: Option<String>,
    pub group_name: Option<String>,
}

impl Metadata {
    pub(crate) fn set_account_names(&mut self, owner: &str, group: &str) {
        fn name(value: &str, id: Option<u32>) -> Option<String> {
            let id = id?;
            (!value.is_empty()
                && value.len() <= 256
                && value != "UNKNOWN"
                && value.parse::<u32>().ok() != Some(id)
                && !value.chars().any(char::is_control))
            .then(|| value.to_owned())
        }
        self.owner_name = name(owner, self.uid);
        self.group_name = name(group, self.gid);
    }
}
#[derive(Clone, Debug)]
pub struct Target {
    pub path: String,
    pub mode: u32,
    pub directory: bool,
}
pub fn parse(value: &str) -> Result<u32> {
    ensure!(
        (3..=4).contains(&value.len()) && value.bytes().all(|c| (b'0'..=b'7').contains(&c)),
        "Enter 3 or 4 octal digits (000–7777)"
    );
    Ok(u32::from_str_radix(value, 8)?)
}
pub fn symbolic(mode: u32) -> String {
    let mut result: Vec<char> = (0..9)
        .map(|i| {
            if mode & (1 << (8 - i)) != 0 {
                ['r', 'w', 'x'][i % 3]
            } else {
                '-'
            }
        })
        .collect();
    for (index, bit, lower, upper) in [
        (2, 0o4000, 's', 'S'),
        (5, 0o2000, 's', 'S'),
        (8, 0o1000, 't', 'T'),
    ] {
        if mode & bit != 0 {
            result[index] = if result[index] == 'x' { lower } else { upper };
        }
    }
    result.into_iter().collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_octal_and_preserves_special_bits() {
        for invalid in ["", "77", "888", " 755", "10000", "-644", "u+rwx"] {
            assert!(parse(invalid).is_err());
        }
        assert_eq!(parse("0755").unwrap(), 0o755);
        assert_eq!(parse("4755").unwrap(), 0o4755);
        assert_eq!(symbolic(0o4755), "rwsr-xr-x");
        assert_eq!(symbolic(0o2644), "rw-r-Sr--");
        assert_eq!(symbolic(0o1777), "rwxrwxrwt");
    }
}
