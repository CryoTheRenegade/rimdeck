use std::collections::HashSet;

/// Steam App ID for RimWorld.
pub const RIMWORLD_STEAM_APP_ID: &str = "294100";

/// Default packageId when About.xml is missing one.
pub const DEFAULT_MISSING_PACKAGEID: &str = "missing.packageid";

/// Official DLC / core package IDs, in typical load order.
pub const RIMWORLD_PACKAGE_IDS: &[&str] = &[
    "ludeon.rimworld",
    "ludeon.rimworld.royalty",
    "ludeon.rimworld.ideology",
    "ludeon.rimworld.biotech",
    "ludeon.rimworld.anomaly",
    "ludeon.rimworld.odyssey",
];

/// RimSort tier-zero mods (Harmony, Prepatcher, Core, DLC).
pub fn known_tier_zero_mods() -> HashSet<String> {
    [
        "zetrith.prepatcher",
        "brrainz.harmony",
        "brrainz.visualexceptions",
        "ludeon.rimworld",
        "ludeon.rimworld.royalty",
        "ludeon.rimworld.ideology",
        "ludeon.rimworld.biotech",
        "ludeon.rimworld.anomaly",
        "ludeon.rimworld.odyssey",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

/// RimSort hardcoded framework / load-near-top mods.
pub fn known_tier_one_mods() -> HashSet<String> {
    [
        "adaptive.storage.framework",
        "aoba.framework",
        "aoba.exosuit.framework",
        "ebsg.framework",
        "imranfish.xmlextensions",
        "thesepeople.ritualattachableoutcomes",
        "ohno.asf.ab.local",
        "oskarpotocki.vanillafactionsexpanded.core",
        "owlchemist.cherrypicker",
        "redmattis.betterprerequisites",
        "smashphil.vehicleframework",
        "unlimitedhugs.hugslib",
        "vanillaexpanded.backgrounds",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

pub fn is_ludeon_package(package_id: &str) -> bool {
    package_id
        .to_ascii_lowercase()
        .starts_with("ludeon.rimworld")
}
