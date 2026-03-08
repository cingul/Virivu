use std::sync::Arc;

use tokio::sync::RwLock;

use crate::models::{FormInvite, MediaUploadTicket, Organization, Project, Site};

#[derive(Debug, Default)]
pub struct InMemoryStore {
    pub organizations: Vec<Organization>,
    pub projects: Vec<Project>,
    pub sites: Vec<Site>,
    pub form_invites: Vec<FormInvite>,
    pub media_tickets: Vec<MediaUploadTicket>,
}

pub type SharedState = Arc<RwLock<InMemoryStore>>;
