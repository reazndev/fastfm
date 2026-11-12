//! `artist.*`, `album.*` and `track.*` read methods, asked about entities from the user's own
//! top lists.

use crate::discover::{Discovered, Entity};
use crate::spec::Spec;

pub(super) fn specs(d: &Discovered) -> Vec<Spec> {
    [artist(d), album(d), track(d)].concat()
}

fn first(list: &[Entity]) -> Entity {
    list.first().cloned().unwrap_or_default()
}

fn with_mbid(list: &[Entity]) -> Option<(Entity, String)> {
    list.iter()
        .find_map(|e| e.mbid.clone().map(|m| (e.clone(), m)))
}

/// Swaps the case of every letter, so `autocorrect` has something to fix.
fn miscased(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_uppercase() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                c.to_uppercase().next().unwrap_or(c)
            }
        })
        .collect()
}

/// Shared `getInfo` variants: plain, with `username=`, autocorrect on and off on a mis-cased
/// name, and by MBID.
fn info_variants(base: &Spec, name_param: &str, name: &str, d: &Discovered) -> Vec<Spec> {
    let miscased = |variant: &str| {
        base.with_variant(variant)
            .replace_param(name_param, &miscased(name))
    };
    vec![
        base.clone(),
        base.with_variant("username").param("username", &d.user),
        miscased("autocorrect-0").num("autocorrect", 0),
        miscased("autocorrect-1").num("autocorrect", 1),
        base.with_variant("lang-de").param("lang", "de"),
    ]
}

fn artist(d: &Discovered) -> Vec<Spec> {
    let top = first(&d.artists);
    let named = |method: &str| Spec::new(method, "default").param("artist", &top.name);
    let info = named("artist.getInfo");
    let mbid = with_mbid(&d.artists).into_iter().flat_map(|(_, mbid)| {
        [
            Spec::new("artist.getInfo", "mbid").param("mbid", &mbid),
            Spec::new("artist.getInfo", "mbid-username")
                .param("mbid", &mbid)
                .param("username", &d.user),
            Spec::new("artist.getSimilar", "mbid").param("mbid", &mbid),
        ]
    });
    let mut specs = info_variants(&info, "artist", &top.name, d);
    specs.extend(mbid);
    specs.extend([
        named("artist.getTags").param("user", &d.user),
        named("artist.getCorrection"),
        Spec::new("artist.getCorrection", "miscased").param("artist", &miscased(&top.name)),
        named("artist.getSimilar"),
        named("artist.getSimilar")
            .with_variant("limit-5")
            .num("limit", 5),
        named("artist.getTopAlbums"),
        named("artist.getTopAlbums")
            .with_variant("page-2")
            .num("page", 2),
        named("artist.getTopTags"),
        named("artist.getTopTracks"),
        named("artist.getTopTracks")
            .with_variant("limit-1")
            .num("limit", 1),
        Spec::new("artist.search", "default").param("artist", &top.name),
        Spec::new("artist.search", "limit-5-page-2")
            .param("artist", &top.name)
            .num("limit", 5)
            .num("page", 2),
    ]);
    specs
}

fn album(d: &Discovered) -> Vec<Spec> {
    let top = first(&d.albums);
    let named = |method: &str| {
        Spec::new(method, "default")
            .param("artist", &top.artist)
            .param("album", &top.name)
    };
    let mut specs = info_variants(&named("album.getInfo"), "album", &top.name, d);
    specs.extend(
        with_mbid(&d.albums)
            .map(|(_, mbid)| Spec::new("album.getInfo", "mbid").param("mbid", &mbid)),
    );
    specs.extend([
        named("album.getTags").param("user", &d.user),
        named("album.getTopTags"),
        Spec::new("album.search", "default").param("album", &top.name),
        Spec::new("album.search", "limit-5")
            .param("album", &top.name)
            .num("limit", 5),
    ]);
    specs
}

fn track(d: &Discovered) -> Vec<Spec> {
    let top = first(&d.tracks);
    let named = |method: &str| {
        Spec::new(method, "default")
            .param("artist", &top.artist)
            .param("track", &top.name)
    };
    let mut specs = info_variants(&named("track.getInfo"), "track", &top.name, d);
    specs.extend(with_mbid(&d.tracks).into_iter().flat_map(|(_, mbid)| {
        [
            Spec::new("track.getInfo", "mbid").param("mbid", &mbid),
            Spec::new("track.getSimilar", "mbid").param("mbid", &mbid),
        ]
    }));
    specs.extend([
        named("track.getTags").param("user", &d.user),
        named("track.getCorrection"),
        named("track.getSimilar"),
        named("track.getSimilar")
            .with_variant("limit-5")
            .num("limit", 5),
        named("track.getTopTags"),
        Spec::new("track.search", "default").param("track", &top.name),
        Spec::new("track.search", "with-artist")
            .param("track", &top.name)
            .param("artist", &top.artist),
    ]);
    specs
}
