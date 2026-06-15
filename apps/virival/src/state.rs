use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::models::{
    CrfSubmission, CrfTemplate, DataQuery, DuaAgreement, Organization, Patient, Site, Study, Visit,
};

#[derive(Debug, Default)]
pub struct Store {
    pub organizations: HashMap<Uuid, Organization>,
    pub studies: HashMap<Uuid, Study>,
    pub sites: HashMap<Uuid, Site>,
    pub patients: HashMap<Uuid, Patient>,
    pub visits: HashMap<Uuid, Visit>,
    pub crf_templates: HashMap<Uuid, CrfTemplate>,
    pub crf_submissions: HashMap<Uuid, CrfSubmission>,
    pub data_queries: HashMap<Uuid, DataQuery>,
    pub duas: HashMap<Uuid, DuaAgreement>,
}

#[derive(Clone, Default)]
pub struct AppState {
    pub store: Arc<RwLock<Store>>,
}
