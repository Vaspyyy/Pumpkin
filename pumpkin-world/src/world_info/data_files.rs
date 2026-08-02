use std::{
    fs::{self, File},
    io::BufWriter,
    path::{Path, PathBuf},
};

use pumpkin_data::game_rules::{GameRule, GameRuleRegistry, GameRuleValue};
use pumpkin_nbt::{
    compound::NbtCompound,
    nbt_compress::{from_gzip_bytes, read_gzip_compound_tag, to_gzip_bytes},
    tag::NbtTag,
};
use pumpkin_util::text::{TextComponent, color::NamedColor, style::Style};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tracing::warn;

use crate::world_info::{WorldGenSettings, WorldInfoError};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct DataFileRoot<T> {
    #[serde(rename = "data")]
    pub data: T,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct WeatherData {
    #[serde(rename = "rain_time", default)]
    pub rain_time: i32,
    #[serde(rename = "raining", default)]
    pub raining: bool,
    #[serde(rename = "thundering", default)]
    pub thundering: bool,
    #[serde(rename = "thunder_time", default)]
    pub thunder_time: i32,
    #[serde(rename = "clear_weather_time", default)]
    pub clear_weather_time: i32,
    #[serde(rename = "DataVersion", default)]
    pub data_version: i32,
}

impl Default for WeatherData {
    fn default() -> Self {
        Self {
            rain_time: 0,
            raining: false,
            thundering: false,
            thunder_time: 0,
            clear_weather_time: -1,
            data_version: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct WorldGenSettingsData {
    #[serde(flatten)]
    pub settings: WorldGenSettings,
    #[serde(rename = "DataVersion", default)]
    pub data_version: i32,
    #[serde(rename = "bonus_chest", default)]
    pub bonus_chest: bool,
    #[serde(rename = "generate_structures", default = "default_true")]
    pub generate_structures: bool,
}

const fn default_true() -> bool {
    true
}

impl WorldGenSettingsData {
    #[must_use]
    pub const fn new(settings: WorldGenSettings, data_version: i32) -> Self {
        Self {
            settings,
            data_version,
            bonus_chest: false,
            generate_structures: true,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DimensionClock {
    pub total_ticks: i64,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct WorldClocksData {
    pub clocks: std::collections::HashMap<String, DimensionClock>,
    pub data_version: i32,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct WanderingTraderData {
    #[serde(rename = "spawn_delay", default = "default_wandering_trader_delay")]
    pub spawn_delay: i32,
    #[serde(rename = "spawn_chance", default = "default_wandering_trader_chance")]
    pub spawn_chance: i32,
    #[serde(rename = "DataVersion", default)]
    pub data_version: i32,
}

const fn default_wandering_trader_delay() -> i32 {
    24_000
}
const fn default_wandering_trader_chance() -> i32 {
    25
}

impl Default for WanderingTraderData {
    fn default() -> Self {
        Self {
            spawn_delay: default_wandering_trader_delay(),
            spawn_chance: default_wandering_trader_chance(),
            data_version: 0,
        }
    }
}

#[must_use]
pub fn minecraft_data_dir(level_folder: &Path) -> PathBuf {
    level_folder.join("data").join("minecraft")
}

/// Ensures the `<world>/data/minecraft/` directory exists.
pub fn ensure_minecraft_data_dir(level_folder: &Path) -> Result<PathBuf, WorldInfoError> {
    let dir = minecraft_data_dir(level_folder);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn read_weather(level_folder: &Path) -> WeatherData {
    let path = minecraft_data_dir(level_folder).join("weather.dat");
    if !path.exists() {
        return WeatherData::default();
    }
    match File::open(&path) {
        Ok(f) => match from_gzip_bytes::<DataFileRoot<WeatherData>, _>(f) {
            Ok(root) => root.data,
            Err(e) => {
                warn!("Failed to deserialize weather.dat, using defaults: {e}");
                WeatherData::default()
            }
        },
        Err(e) => {
            warn!("Failed to open weather.dat, using defaults: {e}");
            WeatherData::default()
        }
    }
}

pub fn write_weather(level_folder: &Path, data: &WeatherData) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("weather.dat");
    let file = File::create(&path)?;
    let root = DataFileRoot { data: data.clone() };
    to_gzip_bytes(&root, BufWriter::new(file))
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

pub fn read_world_gen_settings(level_folder: &Path) -> Option<WorldGenSettings> {
    let path = minecraft_data_dir(level_folder).join("world_gen_settings.dat");
    if !path.exists() {
        return None;
    }
    match File::open(&path) {
        Ok(f) => match from_gzip_bytes::<DataFileRoot<WorldGenSettingsData>, _>(f) {
            Ok(root) => Some(root.data.settings),
            Err(e) => {
                warn!("Failed to deserialize world_gen_settings.dat: {e}");
                None
            }
        },
        Err(e) => {
            warn!("Failed to open world_gen_settings.dat: {e}");
            None
        }
    }
}

pub fn write_world_gen_settings(
    level_folder: &Path,
    settings: &WorldGenSettings,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("world_gen_settings.dat");
    let file = File::create(&path)?;
    let data = WorldGenSettingsData::new(settings.clone(), data_version);
    let root = DataFileRoot { data };
    to_gzip_bytes(&root, BufWriter::new(file))
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

#[must_use]
pub fn game_rules_to_nbt(rules: &GameRuleRegistry, data_version: i32) -> NbtCompound {
    let mut inner = NbtCompound::new();
    for rule in GameRule::all() {
        let key = format!("minecraft:{rule}");
        match rules.get(rule) {
            GameRuleValue::Bool(b) => inner.put(&key, NbtTag::Byte(i8::from(*b))),
            GameRuleValue::Int(i) => inner.put(&key, NbtTag::Int(*i as i32)),
        }
    }
    inner.put_int("DataVersion", data_version);

    let mut root = NbtCompound::new();
    root.put_compound("data", inner);
    root
}

pub fn game_rules_from_nbt(root: &NbtCompound) -> GameRuleRegistry {
    let mut registry = GameRuleRegistry::default();

    let Some(inner) = root.get_compound("data") else {
        warn!("game_rules.dat missing 'data' compound, using defaults");
        return registry;
    };

    for rule in GameRule::all() {
        let key = format!("minecraft:{rule}");
        match registry.get_mut(rule) {
            GameRuleValue::Bool(b) => {
                if let Some(v) = inner.get_byte(&key) {
                    *b = v != 0;
                }
            }
            GameRuleValue::Int(i) => {
                if let Some(v) = inner.get_int(&key) {
                    *i = i64::from(v);
                }
            }
        }
    }

    registry
}

pub fn read_game_rules(level_folder: &Path) -> GameRuleRegistry {
    let path = minecraft_data_dir(level_folder).join("game_rules.dat");
    if !path.exists() {
        return GameRuleRegistry::default();
    }

    match File::open(&path) {
        Ok(f) => match read_gzip_compound_tag(f) {
            Ok(compound) => game_rules_from_nbt(&compound),
            Err(e) => {
                warn!("Failed to parse game_rules.dat: {e}");
                GameRuleRegistry::default()
            }
        },
        Err(e) => {
            warn!("Failed to open game_rules.dat: {e}");
            GameRuleRegistry::default()
        }
    }
}

pub fn write_game_rules(
    level_folder: &Path,
    rules: &GameRuleRegistry,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("game_rules.dat");

    let compound = game_rules_to_nbt(rules, data_version);
    let file = File::create(&path)?;

    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(compound, file)
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

pub fn read_world_clocks(level_folder: &Path) -> WorldClocksData {
    let path = minecraft_data_dir(level_folder).join("world_clocks.dat");
    if !path.exists() {
        return WorldClocksData::default();
    }

    match File::open(&path) {
        Ok(f) => match read_gzip_compound_tag(f) {
            Ok(compound) => world_clocks_from_nbt(&compound),
            Err(e) => {
                warn!("Failed to parse world_clocks.dat: {e}");
                WorldClocksData::default()
            }
        },
        Err(e) => {
            warn!("Failed to open world_clocks.dat: {e}");
            WorldClocksData::default()
        }
    }
}

fn world_clocks_from_nbt(root: &NbtCompound) -> WorldClocksData {
    let mut result = WorldClocksData::default();

    let Some(inner) = root.get_compound("data") else {
        return result;
    };

    result.data_version = inner.get_int("DataVersion").unwrap_or(0);

    for (key, tag) in &inner.child_tags {
        if key.as_ref() == "DataVersion" {
            continue;
        }
        if let NbtTag::Compound(dim_compound) = tag {
            let total_ticks = dim_compound.get_long("total_ticks").unwrap_or(0);
            result
                .clocks
                .insert(key.to_string(), DimensionClock { total_ticks });
        }
    }

    result
}

pub fn write_world_clocks(
    level_folder: &Path,
    clocks: &WorldClocksData,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("world_clocks.dat");

    let mut inner = NbtCompound::new();
    for (dim_name, clock) in &clocks.clocks {
        let mut dim_compound = NbtCompound::new();
        dim_compound.put_long("total_ticks", clock.total_ticks);
        inner.put_compound(dim_name, dim_compound);
    }
    inner.put_int("DataVersion", clocks.data_version);

    let mut root = NbtCompound::new();
    root.put_compound("data", inner);

    let file = File::create(&path)?;

    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, file)
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

pub fn read_wandering_trader(level_folder: &Path) -> WanderingTraderData {
    let path = minecraft_data_dir(level_folder).join("wandering_trader.dat");
    if !path.exists() {
        return WanderingTraderData::default();
    }
    match File::open(&path) {
        Ok(f) => match from_gzip_bytes::<DataFileRoot<WanderingTraderData>, _>(f) {
            Ok(root) => root.data,
            Err(e) => {
                warn!("Failed to deserialize wandering_trader.dat, using defaults: {e}");
                WanderingTraderData::default()
            }
        },
        Err(e) => {
            warn!("Failed to open wandering_trader.dat: {e}");
            WanderingTraderData::default()
        }
    }
}

pub fn write_wandering_trader(
    level_folder: &Path,
    data: &WanderingTraderData,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("wandering_trader.dat");
    let file = File::create(&path)?;
    let root = DataFileRoot { data: data.clone() };
    to_gzip_bytes(&root, BufWriter::new(file))
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

pub fn write_custom_boss_events_stub(
    level_folder: &Path,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("custom_boss_events.dat");
    // Only create if absent; actual boss-bar persistence lives elsewhere.
    if path.exists() {
        return Ok(());
    }

    let mut inner = NbtCompound::new();
    inner.put_int("DataVersion", data_version);
    let mut root = NbtCompound::new();
    root.put_compound("data", inner);

    let file = File::create(&path)?;

    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, file)
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

pub fn write_scheduled_events_stub(
    level_folder: &Path,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("scheduled_events.dat");
    if path.exists() {
        return Ok(());
    }

    let mut inner = NbtCompound::new();
    inner.put("events", NbtTag::List(vec![]));
    inner.put_int("DataVersion", data_version);
    let mut root = NbtCompound::new();
    root.put_compound("data", inner);

    let file = File::create(&path)?;

    pumpkin_nbt::nbt_compress::write_gzip_compound_tag(root, file)
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

/// Serializable scoreboard data for `data/minecraft/scoreboard.dat`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ScoreboardData {
    #[serde(rename = "Objectives", alias = "objectives", default)]
    pub objectives: Vec<SerializableObjective>,
    #[serde(rename = "PlayerScores", alias = "scores", default)]
    pub scores: Vec<SerializableScore>,
    #[serde(rename = "Teams", alias = "teams", default)]
    pub teams: Vec<SerializableTeam>,
    /// Display slot bindings: slot name (e.g. "list", "sidebar") -> objective name
    #[serde(rename = "DisplaySlots", alias = "displaySlots", default)]
    pub display_slots: std::collections::HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SerializableObjective {
    #[serde(rename = "Name", alias = "name")]
    pub name: String,
    #[serde(
        rename = "DisplayName",
        alias = "displayName",
        default = "empty_text_component"
    )]
    pub display_name: SerializableTextComponent,
    #[serde(
        rename = "RenderType",
        alias = "renderType",
        default = "default_render_type"
    )]
    pub render_type: String,
    #[serde(
        rename = "CriteriaName",
        alias = "criteriaName",
        default = "default_criteria_name"
    )]
    pub criteria_name: String,
    /// Whether the score's display name is auto-updated when the score changes.
    #[serde(rename = "display_auto_update", alias = "displayAutoUpdate", default)]
    pub display_auto_update: bool,
    /// Optional number format for all scores in this objective.
    #[serde(rename = "format", default, skip_serializing_if = "Option::is_none")]
    pub number_format: Option<SerializableNumberFormat>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SerializableScore {
    #[serde(rename = "Name", alias = "entityName")]
    pub entity_name: String,
    #[serde(rename = "Objective", alias = "objectiveName")]
    pub objective_name: String,
    #[serde(rename = "Score", alias = "value")]
    pub value: i32,
    #[serde(rename = "Locked", alias = "locked", default = "default_true")]
    pub locked: bool,
    /// Optional per-score display name.
    #[serde(rename = "display", default, skip_serializing_if = "Option::is_none")]
    pub display: Option<SerializableTextComponent>,
    /// Optional per-score number format override.
    #[serde(rename = "format", default, skip_serializing_if = "Option::is_none")]
    pub number_format: Option<SerializableNumberFormat>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SerializableTeam {
    #[serde(rename = "Name", alias = "name")]
    pub name: String,
    #[serde(
        rename = "DisplayName",
        alias = "displayName",
        default = "empty_text_component"
    )]
    pub display_name: SerializableTextComponent,
    #[serde(rename = "TeamColor", default = "default_team_color")]
    pub color: NamedColor,
    #[serde(rename = "AllowFriendlyFire", default)]
    pub allow_friendly_fire: bool,
    #[serde(rename = "SeeFriendlyInvisibles", default)]
    pub see_friendly_invisibles: bool,
    #[serde(rename = "MemberNamePrefix", default = "empty_text_component")]
    pub player_prefix: SerializableTextComponent,
    #[serde(rename = "MemberNameSuffix", default = "empty_text_component")]
    pub player_suffix: SerializableTextComponent,
    #[serde(rename = "NameTagVisibility", default = "default_visibility")]
    pub nametag_visibility: String,
    #[serde(rename = "DeathMessageVisibility", default = "default_visibility")]
    pub death_message_visibility: String,
    #[serde(rename = "CollisionRule", default = "default_collision_rule")]
    pub collision_rule: String,
    #[serde(rename = "Players", alias = "players", default)]
    pub players: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SerializableNumberFormat {
    Blank,
    Styled { style: Style },
    Fixed { value: SerializableTextComponent },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerializableTextComponent(pub TextComponent);

impl Serialize for SerializableTextComponent {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SerializableTextComponent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        TextComponent::deserialize(deserializer).map(Self)
    }
}

fn empty_text_component() -> SerializableTextComponent {
    SerializableTextComponent(TextComponent::empty())
}

const fn default_team_color() -> NamedColor {
    NamedColor::White
}

fn default_visibility() -> String {
    "always".to_string()
}

fn default_render_type() -> String {
    "integer".to_string()
}

fn default_criteria_name() -> String {
    "dummy".to_string()
}

fn default_collision_rule() -> String {
    "always".to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct ScoreboardDataFileRoot {
    #[serde(rename = "data")]
    data: ScoreboardData,
    #[serde(rename = "DataVersion", default)]
    data_version: i32,
}

pub fn read_scoreboard(level_folder: &Path) -> ScoreboardData {
    let path = minecraft_data_dir(level_folder).join("scoreboard.dat");
    if !path.exists() {
        return ScoreboardData::default();
    }
    match File::open(&path) {
        Ok(f) => match from_gzip_bytes::<ScoreboardDataFileRoot, _>(f) {
            Ok(root) => root.data,
            Err(e) => {
                warn!("Failed to deserialize scoreboard.dat, using defaults: {e}");
                ScoreboardData::default()
            }
        },
        Err(e) => {
            warn!("Failed to open scoreboard.dat, using defaults: {e}");
            ScoreboardData::default()
        }
    }
}

pub fn write_scoreboard(
    level_folder: &Path,
    data: &ScoreboardData,
    data_version: i32,
) -> Result<(), WorldInfoError> {
    let dir = ensure_minecraft_data_dir(level_folder)?;
    let path = dir.join("scoreboard.dat");
    let file = File::create(&path)?;
    let root = ScoreboardDataFileRoot {
        data: data.clone(),
        data_version,
    };
    to_gzip_bytes(&root, BufWriter::new(file))
        .map_err(|e| WorldInfoError::SerializationError(e.to_string()))
}

#[cfg(test)]
mod scoreboard_tests {
    use super::*;

    #[test]
    fn scoreboard_uses_vanilla_nbt_names_and_round_trips() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let data = ScoreboardData {
            objectives: vec![SerializableObjective {
                name: "test".to_string(),
                display_name: SerializableTextComponent(TextComponent::text("Test")),
                render_type: "integer".to_string(),
                criteria_name: "dummy".to_string(),
                display_auto_update: true,
                number_format: Some(SerializableNumberFormat::Blank),
            }],
            scores: vec![SerializableScore {
                entity_name: "#constant".to_string(),
                objective_name: "test".to_string(),
                value: 7,
                locked: true,
                display: Some(SerializableTextComponent(TextComponent::text("Seven"))),
                number_format: None,
            }],
            teams: vec![SerializableTeam {
                name: "green".to_string(),
                display_name: SerializableTextComponent(TextComponent::text("Green Team")),
                color: NamedColor::Green,
                allow_friendly_fire: true,
                see_friendly_invisibles: true,
                player_prefix: SerializableTextComponent(TextComponent::text("[G] ")),
                player_suffix: SerializableTextComponent(TextComponent::empty()),
                nametag_visibility: "always".to_string(),
                death_message_visibility: "never".to_string(),
                collision_rule: "pushOtherTeams".to_string(),
                players: vec!["Player".to_string()],
            }],
            display_slots: std::iter::once(("sidebar".to_string(), "test".to_string())).collect(),
        };

        write_scoreboard(temporary_directory.path(), &data, 4903).expect("write scoreboard data");

        let path = minecraft_data_dir(temporary_directory.path()).join("scoreboard.dat");
        let root = read_gzip_compound_tag(File::open(path).expect("open scoreboard data"))
            .expect("read scoreboard NBT");
        assert_eq!(root.get_int("DataVersion"), Some(4903));
        let inner = root.get_compound("data").expect("data compound");
        assert!(inner.has("Objectives"));
        assert!(inner.has("PlayerScores"));
        assert!(inner.has("DisplaySlots"));
        assert!(inner.has("Teams"));
        assert!(!inner.has("objectives"));
        assert!(!inner.has("scores"));

        let NbtTag::Compound(objective) =
            &inner.get_list("Objectives").expect("objectives list")[0]
        else {
            panic!("objective was not a compound");
        };
        assert_eq!(objective.get_string("Name"), Some("test"));
        assert_eq!(objective.get_string("CriteriaName"), Some("dummy"));
        assert!(objective.has("DisplayName"));
        assert!(objective.has("display_auto_update"));

        let NbtTag::Compound(score) = &inner.get_list("PlayerScores").expect("scores list")[0]
        else {
            panic!("score was not a compound");
        };
        assert_eq!(score.get_string("Name"), Some("#constant"));
        assert_eq!(score.get_string("Objective"), Some("test"));
        assert_eq!(score.get_int("Score"), Some(7));
        assert_eq!(score.get_byte("Locked"), Some(1));

        assert_eq!(read_scoreboard(temporary_directory.path()), data);
    }
}
