use std::fmt;

pub enum GitError {
    NotARepo,
    GitNotFound,
    CommandFailed(String),
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitError::NotARepo => write!(f, "not a git repository"),
            GitError::GitNotFound => write!(f, "git binary not found on PATH"),
            GitError::CommandFailed(msg) => write!(f, "git command failed: {msg}"),
        }
    }
}

impl From<GitError> for String {
    fn from(err: GitError) -> Self {
        err.to_string()
    }
}
