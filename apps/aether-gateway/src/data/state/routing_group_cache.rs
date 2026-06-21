use std::sync::Arc;
use std::time::Duration;

use aether_cache::ExpiringMap;
use aether_data::DataLayerError;
use aether_data_contracts::repository::routing_profiles::{
    RoutingGroupBindingQuery, RoutingGroupBindingSubject, RoutingGroupLookupKey,
    RoutingGroupReadRepository, StoredRoutingGroup, StoredRoutingGroupBinding,
    StoredRoutingGroupVersion,
};
use async_trait::async_trait;

const ROUTING_GROUP_CACHE_TTL: Duration = Duration::from_secs(5);
const ROUTING_GROUP_CACHE_MAX_ENTRIES: usize = 4_096;

pub(super) struct CachedRoutingGroupReadRepository {
    inner: Arc<dyn RoutingGroupReadRepository>,
    entries: ExpiringMap<RoutingGroupCacheKey, RoutingGroupCacheValue>,
    load_guard: tokio::sync::Mutex<()>,
}

impl CachedRoutingGroupReadRepository {
    pub(super) fn new(inner: Arc<dyn RoutingGroupReadRepository>) -> Self {
        Self {
            inner,
            entries: ExpiringMap::new(),
            load_guard: tokio::sync::Mutex::new(()),
        }
    }

    async fn get_or_load(
        &self,
        key: RoutingGroupCacheKey,
        load: impl std::future::Future<Output = Result<RoutingGroupCacheValue, DataLayerError>>,
    ) -> Result<RoutingGroupCacheValue, DataLayerError> {
        if let Some(value) = self.entries.get_fresh(&key, ROUTING_GROUP_CACHE_TTL) {
            return Ok(value);
        }
        let _guard = self.load_guard.lock().await;
        if let Some(value) = self.entries.get_fresh(&key, ROUTING_GROUP_CACHE_TTL) {
            return Ok(value);
        }
        let value = load.await?;
        self.entries.insert(
            key,
            value.clone(),
            ROUTING_GROUP_CACHE_TTL,
            ROUTING_GROUP_CACHE_MAX_ENTRIES,
        );
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RoutingGroupCacheKey {
    ListGroups,
    FindById(String),
    FindByName(String),
    FindSystemDefault,
    Bindings {
        group_id: Option<String>,
        subject_type: Option<&'static str>,
        subject_id: Option<String>,
    },
    Versions(String),
}

#[derive(Debug, Clone)]
enum RoutingGroupCacheValue {
    Groups(Vec<StoredRoutingGroup>),
    Group(Option<StoredRoutingGroup>),
    Bindings(Vec<StoredRoutingGroupBinding>),
    Versions(Vec<StoredRoutingGroupVersion>),
}

fn lookup_cache_key(lookup: &RoutingGroupLookupKey<'_>) -> RoutingGroupCacheKey {
    match lookup {
        RoutingGroupLookupKey::Id(id) => RoutingGroupCacheKey::FindById((*id).to_string()),
        RoutingGroupLookupKey::Name(name) => RoutingGroupCacheKey::FindByName((*name).to_string()),
        RoutingGroupLookupKey::SystemDefault => RoutingGroupCacheKey::FindSystemDefault,
    }
}

fn subject_cache_key(subject: Option<RoutingGroupBindingSubject>) -> Option<&'static str> {
    match subject {
        Some(RoutingGroupBindingSubject::User) => Some("user"),
        Some(RoutingGroupBindingSubject::ApiKey) => Some("api_key"),
        Some(RoutingGroupBindingSubject::UserGroup) => Some("user_group"),
        None => None,
    }
}

#[async_trait]
impl RoutingGroupReadRepository for CachedRoutingGroupReadRepository {
    async fn list_routing_groups(&self) -> Result<Vec<StoredRoutingGroup>, DataLayerError> {
        match self
            .get_or_load(RoutingGroupCacheKey::ListGroups, async {
                self.inner
                    .list_routing_groups()
                    .await
                    .map(RoutingGroupCacheValue::Groups)
            })
            .await?
        {
            RoutingGroupCacheValue::Groups(groups) => Ok(groups),
            _ => Ok(Vec::new()),
        }
    }

    async fn find_routing_group(
        &self,
        lookup: RoutingGroupLookupKey<'_>,
    ) -> Result<Option<StoredRoutingGroup>, DataLayerError> {
        let key = lookup_cache_key(&lookup);
        match self
            .get_or_load(key, async {
                self.inner
                    .find_routing_group(lookup)
                    .await
                    .map(RoutingGroupCacheValue::Group)
            })
            .await?
        {
            RoutingGroupCacheValue::Group(group) => Ok(group),
            _ => Ok(None),
        }
    }

    async fn list_routing_group_bindings(
        &self,
        query: &RoutingGroupBindingQuery,
    ) -> Result<Vec<StoredRoutingGroupBinding>, DataLayerError> {
        let key = RoutingGroupCacheKey::Bindings {
            group_id: query.group_id.clone(),
            subject_type: subject_cache_key(query.subject_type),
            subject_id: query.subject_id.clone(),
        };
        match self
            .get_or_load(key, async {
                self.inner
                    .list_routing_group_bindings(query)
                    .await
                    .map(RoutingGroupCacheValue::Bindings)
            })
            .await?
        {
            RoutingGroupCacheValue::Bindings(bindings) => Ok(bindings),
            _ => Ok(Vec::new()),
        }
    }

    async fn list_routing_group_versions(
        &self,
        group_id: &str,
    ) -> Result<Vec<StoredRoutingGroupVersion>, DataLayerError> {
        let key = RoutingGroupCacheKey::Versions(group_id.to_string());
        match self
            .get_or_load(key, async {
                self.inner
                    .list_routing_group_versions(group_id)
                    .await
                    .map(RoutingGroupCacheValue::Versions)
            })
            .await?
        {
            RoutingGroupCacheValue::Versions(versions) => Ok(versions),
            _ => Ok(Vec::new()),
        }
    }
}
