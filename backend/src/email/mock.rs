use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;

use super::{MailError, Mailer};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedEmail {
    pub sender_name: String,
    pub sender_email: String,
    pub to_email: String,
    pub to_name: String,
    pub subject: String,
    pub html: String,
}

static INBOX: OnceLock<Arc<Mutex<Vec<RecordedEmail>>>> = OnceLock::new();

pub fn mock_inbox() -> Arc<Mutex<Vec<RecordedEmail>>> {
    INBOX
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

pub fn clear_mock_inbox() {
    if let Some(inbox) = INBOX.get() {
        let mut guard = inbox.lock().expect("mock inbox lock");
        guard.clear();
    }
}

pub fn take_mock_emails() -> Vec<RecordedEmail> {
    let inbox = mock_inbox();
    let mut guard = inbox.lock().expect("mock inbox lock");
    std::mem::take(&mut *guard)
}

#[derive(Clone)]
pub struct MockMailer {
    inbox: Arc<Mutex<Vec<RecordedEmail>>>,
}

impl Default for MockMailer {
    fn default() -> Self {
        Self {
            inbox: mock_inbox(),
        }
    }
}

impl MockMailer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn arc() -> Arc<dyn Mailer> {
        Arc::new(Self::new())
    }
}

#[async_trait]
impl Mailer for MockMailer {
    async fn send(
        &self,
        sender_name: &str,
        sender_email: &str,
        to_email: &str,
        to_name: &str,
        subject: &str,
        html: &str,
    ) -> Result<(), MailError> {
        let mut guard = self.inbox.lock().expect("mock inbox lock");
        guard.push(RecordedEmail {
            sender_name: sender_name.to_string(),
            sender_email: sender_email.to_string(),
            to_email: to_email.to_string(),
            to_name: to_name.to_string(),
            subject: subject.to_string(),
            html: html.to_string(),
        });
        Ok(())
    }
}
