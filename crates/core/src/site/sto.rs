//! serienstream.to (SerienStream) — dünne Wrapper um [`super::serienstream`].

use crate::error::Result;
use crate::model::{Episode, ProviderLinks, BASE_STO};

pub async fn list_seasons(client: &reqwest::Client, slug: &str) -> Result<Vec<u32>> {
    // serienstream.to nutzt `/serie/<slug>` (ohne `/stream/`) mit href-basierter Liste.
    let url = format!("{BASE_STO}/serie/{slug}");
    let body = super::fetch_text(client, &url).await?;
    Ok(super::serienstream::parse_seasons_sto(&body, slug))
}

pub async fn list_episodes(
    client: &reqwest::Client,
    slug: &str,
    season: u32,
) -> Result<Vec<Episode>> {
    let url = format!("{BASE_STO}/serie/{slug}/staffel-{season}");
    let body = super::fetch_text(client, &url).await?;
    Ok(super::serienstream::parse_episodes_sto(
        &body, BASE_STO, slug, season,
    ))
}

pub async fn provider_links(client: &reqwest::Client, episode_url: &str) -> Result<ProviderLinks> {
    super::serienstream::provider_links(client, BASE_STO, episode_url).await
}
