//! `tag.*`, `chart.*` and `geo.*`: cached pass-through methods.

use crate::discover::Discovered;
use crate::spec::Spec;

pub(super) fn specs(d: &Discovered) -> Vec<Spec> {
    [tag(&d.global_tag), chart(), geo()].concat()
}

fn tag(tag: &str) -> Vec<Spec> {
    let named = |method: &str| Spec::new(method, "default").param("tag", tag);
    vec![
        named("tag.getInfo"),
        named("tag.getInfo")
            .with_variant("lang-de")
            .param("lang", "de"),
        named("tag.getSimilar"),
        named("tag.getTopAlbums"),
        named("tag.getTopAlbums")
            .with_variant("limit-1")
            .num("limit", 1),
        named("tag.getTopArtists"),
        named("tag.getTopArtists")
            .with_variant("page-2")
            .num("page", 2),
        named("tag.getTopTracks"),
        named("tag.getWeeklyChartList"),
        Spec::new("tag.getTopTags", "default"),
    ]
}

fn chart() -> Vec<Spec> {
    [
        "chart.getTopArtists",
        "chart.getTopTags",
        "chart.getTopTracks",
    ]
    .into_iter()
    .flat_map(|method| {
        let base = Spec::new(method, "default");
        [
            base.with_variant("limit-1").num("limit", 1),
            base.with_variant("page-2").num("page", 2),
            base,
        ]
    })
    .collect()
}

fn geo() -> Vec<Spec> {
    ["geo.getTopArtists", "geo.getTopTracks"]
        .into_iter()
        .flat_map(|method| {
            let base = Spec::new(method, "default").param("country", "Germany");
            [
                base.with_variant("limit-1").num("limit", 1),
                base.with_variant("lowercase-country")
                    .replace_param("country", "germany"),
                Spec::new(method, "error-unknown-country").param("country", "Atlantis"),
                Spec::new(method, "error-missing-country"),
                base,
            ]
        })
        .collect()
}
