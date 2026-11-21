//! The `user@host[:port]` argument.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl Target {
    /// Parses `[user@]host[:port]`; `host` may be `[v6addr]` or a bare IPv6
    /// address without a port. A missing user falls back to `default_user`.
    pub fn parse(s: &str, default_user: Option<String>) -> Result<Self, String> {
        let (user, rest) = match s.rsplit_once('@') {
            Some((user, rest)) => (Some(user.to_string()), rest),
            None => (default_user, s),
        };
        let user = user
            .filter(|u| !u.is_empty())
            .ok_or_else(|| format!("no user name in {s:?} (use user@host)"))?;
        let (host, port) = if let Some(v6) = rest.strip_prefix('[') {
            let (host, after) = v6
                .split_once(']')
                .ok_or_else(|| format!("missing ']' in {s:?}"))?;
            match after {
                "" => (host, None),
                _ => (
                    host,
                    Some(
                        after
                            .strip_prefix(':')
                            .ok_or_else(|| format!("unexpected {after:?} after ']'"))?,
                    ),
                ),
            }
        } else {
            match rest.split_once(':') {
                Some((host, port)) if !port.contains(':') => (host, Some(port)),
                _ => (rest, None),
            }
        };
        if host.is_empty() {
            return Err(format!("no host in {s:?}"));
        }
        let port = match port {
            Some(p) => p.parse().map_err(|_| format!("bad port {p:?}"))?,
            None => 22,
        };
        Ok(Self {
            user,
            host: host.to_string(),
            port,
        })
    }
}

impl FromStr for Target {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .ok();
        Self::parse(s, user)
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.port, self.host.contains(':')) {
            (22, _) => write!(f, "{}@{}", self.user, self.host),
            (port, true) => write!(f, "{}@[{}]:{port}", self.user, self.host),
            (port, false) => write!(f, "{}@{}:{port}", self.user, self.host),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Target, String> {
        Target::parse(s, Some("me".into()))
    }

    fn target(user: &str, host: &str, port: u16) -> Target {
        Target {
            user: user.into(),
            host: host.into(),
            port,
        }
    }

    #[test]
    fn user_host_and_port_are_split() {
        assert_eq!(
            parse("alice@example.com"),
            Ok(target("alice", "example.com", 22))
        );
        assert_eq!(
            parse("alice@127.0.0.1:2222"),
            Ok(target("alice", "127.0.0.1", 2222))
        );
    }

    #[test]
    fn missing_user_uses_the_default() {
        assert_eq!(parse("example.com"), Ok(target("me", "example.com", 22)));
        assert!(Target::parse("example.com", None).is_err());
    }

    #[test]
    fn the_last_at_sign_separates_the_user() {
        assert_eq!(parse("a@b@host"), Ok(target("a@b", "host", 22)));
    }

    #[test]
    fn ipv6_hosts_take_a_port_only_in_brackets() {
        assert_eq!(parse("u@[::1]:2222"), Ok(target("u", "::1", 2222)));
        assert_eq!(parse("u@[::1]"), Ok(target("u", "::1", 22)));
        assert_eq!(parse("u@fe80::1"), Ok(target("u", "fe80::1", 22)));
    }

    #[test]
    fn malformed_targets_are_rejected() {
        for s in ["u@", "u@:22", "u@host:x", "u@host:70000", "u@[::1", "@host"] {
            assert!(parse(s).is_err(), "{s}");
        }
    }

    #[test]
    fn display_round_trips() {
        for s in ["u@host", "u@host:2222", "u@[::1]:2222"] {
            assert_eq!(parse(s).unwrap().to_string(), s);
        }
    }
}
