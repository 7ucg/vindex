//! serienstream.to (SerienStream) — dünne Wrapper um [`super::serienstream`].

use crate::error::Result;
use crate::model::{Episode, ProviderLinks, BASE_STO};

pub async fn list_seasons(client: &reqwest::Client, slug: &str) -> Result<Vec<u32>> {
    super::serienstream::list_seasons(client, BASE_STO, slug).await
}

pub async fn list_episodes(
    client: &reqwest::Client,
    slug: &str,
    season: u32,
) -> Result<Vec<Episode>> {
    super::serienstream::list_episodes(client, BASE_STO, slug, season).await
}

pub async fn provider_links(client: &reqwest::Client, episode_url: &str) -> Result<ProviderLinks> {
    super::serienstream::provider_links(client, BASE_STO, episode_url).await
}
