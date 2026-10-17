use crate::{
    domain::{
        model::Identity,
        repo::Repos,
        vo::{AccountId, IdentityId},
    },
    error::Result,
};

#[derive(Debug)]
pub struct IdentityService {
    repos: Repos,
}

impl IdentityService {
    pub fn new(repos: Repos) -> Self {
        Self { repos }
    }

    pub async fn create_identity(&self, name: Option<&str>) -> Result<Identity> {
        let now = jiff::Timestamp::now();
        self.repos
            .identity
            .create(IdentityId(uuid::Uuid::now_v7()), name, None, now, now)
            .await
    }

    pub async fn bind_account(&self, identity_id: IdentityId, account_id: AccountId) -> Result<()> {
        self.repos
            .account
            .bind_identity(account_id, identity_id)
            .await
    }
}
