use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pumpkin_data::scoreboard::ScoreboardDisplaySlot;
use pumpkin_protocol::{
    BClientPacket, ClientPacket, NumberFormat,
    bedrock::client::scoreboard::{CSetScore as BSetScore, ScoreEntry as BScoreEntry},
    codec::var_int::VarInt,
    java::client::play::{
        CDisplayObjective, CResetScore, CSetPlayerTeam, CUpdateObjectives, CUpdateScore, Mode,
        RenderType, TeamMethod, TeamParameters,
    },
};
use pumpkin_util::text::{TextComponent, color::NamedColor};
use tracing::warn;

use super::World;

/// Returns `true` if the criterion name is valid (either a known built-in,
/// a modern statistic criterion, or a team-kill criterion).
#[must_use]
pub fn is_valid_criterion(criterion: &str) -> bool {
    const STAT_TYPES: &[&str] = &[
        "minecraft.mined",
        "minecraft.crafted",
        "minecraft.used",
        "minecraft.broken",
        "minecraft.picked_up",
        "minecraft.dropped",
        "minecraft.killed",
        "minecraft.killed_by",
        "minecraft.custom",
    ];
    const BUILT_IN_CRITERIA: &[&str] = &[
        "dummy",
        "trigger",
        "deathCount",
        "playerKillCount",
        "totalKillCount",
        "health",
        "food",
        "air",
        "armor",
        "xp",
        "level",
    ];
    const TEAM_COLORS: &[&str] = &[
        "black",
        "dark_blue",
        "dark_green",
        "dark_aqua",
        "dark_red",
        "dark_purple",
        "gold",
        "gray",
        "dark_gray",
        "blue",
        "green",
        "aqua",
        "red",
        "light_purple",
        "yellow",
        "white",
    ];

    if BUILT_IN_CRITERIA.contains(&criterion) {
        return true;
    }

    // Team kill criteria: teamkill.<color> or killedByTeam.<color>
    for color in TEAM_COLORS {
        let teamkill_crit = format!("teamkill.{color}");
        let killed_crit = format!("killedByTeam.{color}");
        if criterion == teamkill_crit || criterion == killed_crit {
            return true;
        }
    }

    let Some((stat_type, stat)) = criterion.split_once(':') else {
        return false;
    };
    STAT_TYPES.contains(&stat_type) && is_encoded_resource_location(stat)
}

fn is_encoded_resource_location(value: &str) -> bool {
    value.split_once('.').is_some_and(|(namespace, path)| {
        !namespace.is_empty()
            && !path.is_empty()
            && namespace.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte)
            })
            && path.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-/.".contains(&byte)
            })
    })
}

fn pascal_case_to_snake_case(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 4);
    for (index, character) in value.chars().enumerate() {
        if character.is_ascii_uppercase() {
            if index != 0 {
                result.push('_');
            }
            result.push(character.to_ascii_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}

/// Converts a statistic update to the criterion name stored by vanilla.
///
/// Item, block, and entity statistic names need their registry identifiers;
/// custom statistics can be mapped directly from their generated enum.
#[must_use]
pub fn statistic_criterion(
    category: pumpkin_data::statistic::StatisticCategory,
    statistic: i32,
) -> Option<String> {
    use pumpkin_data::statistic::{CustomStatistic, StatisticCategory};

    if category != StatisticCategory::Custom {
        return None;
    }
    let statistic = CustomStatistic::from_i32(statistic)?;
    Some(format!(
        "minecraft.custom:minecraft.{}",
        pascal_case_to_snake_case(&format!("{statistic:?}"))
    ))
}

/// Returns the default [`RenderType`] for a built-in criterion.
///
/// Only `"health"` defaults to `Hearts`; all others default to `Integer`.
#[must_use]
pub fn default_render_type_for_criterion(criterion: &str) -> RenderType {
    match criterion {
        "health" => RenderType::Hearts,
        _ => RenderType::Integer,
    }
}

/// Returns the string name for a display slot.
/// Used as the NBT key in `scoreboard.dat` (NBT only supports string keys).
#[must_use]
pub const fn display_slot_name(slot: ScoreboardDisplaySlot) -> &'static str {
    match slot {
        ScoreboardDisplaySlot::List => "list",
        ScoreboardDisplaySlot::Sidebar => "sidebar",
        ScoreboardDisplaySlot::BelowName => "below_name",
        ScoreboardDisplaySlot::TeamBlack => "sidebar.team.black",
        ScoreboardDisplaySlot::TeamDarkBlue => "sidebar.team.dark_blue",
        ScoreboardDisplaySlot::TeamDarkGreen => "sidebar.team.dark_green",
        ScoreboardDisplaySlot::TeamDarkAqua => "sidebar.team.dark_aqua",
        ScoreboardDisplaySlot::TeamDarkRed => "sidebar.team.dark_red",
        ScoreboardDisplaySlot::TeamDarkPurple => "sidebar.team.dark_purple",
        ScoreboardDisplaySlot::TeamGold => "sidebar.team.gold",
        ScoreboardDisplaySlot::TeamGray => "sidebar.team.gray",
        ScoreboardDisplaySlot::TeamDarkGray => "sidebar.team.dark_gray",
        ScoreboardDisplaySlot::TeamBlue => "sidebar.team.blue",
        ScoreboardDisplaySlot::TeamGreen => "sidebar.team.green",
        ScoreboardDisplaySlot::TeamAqua => "sidebar.team.aqua",
        ScoreboardDisplaySlot::TeamRed => "sidebar.team.red",
        ScoreboardDisplaySlot::TeamLightPurple => "sidebar.team.light_purple",
        ScoreboardDisplaySlot::TeamYellow => "sidebar.team.yellow",
        ScoreboardDisplaySlot::TeamWhite => "sidebar.team.white",
    }
}

/// Parses a display slot from its string name (inverse of [`display_slot_name`]).
#[must_use]
pub fn display_slot_from_name(name: &str) -> Option<ScoreboardDisplaySlot> {
    match name {
        "list" => Some(ScoreboardDisplaySlot::List),
        "sidebar" => Some(ScoreboardDisplaySlot::Sidebar),
        "below_name" => Some(ScoreboardDisplaySlot::BelowName),
        "sidebar.team.black" => Some(ScoreboardDisplaySlot::TeamBlack),
        "sidebar.team.dark_blue" => Some(ScoreboardDisplaySlot::TeamDarkBlue),
        "sidebar.team.dark_green" => Some(ScoreboardDisplaySlot::TeamDarkGreen),
        "sidebar.team.dark_aqua" => Some(ScoreboardDisplaySlot::TeamDarkAqua),
        "sidebar.team.dark_red" => Some(ScoreboardDisplaySlot::TeamDarkRed),
        "sidebar.team.dark_purple" => Some(ScoreboardDisplaySlot::TeamDarkPurple),
        "sidebar.team.gold" => Some(ScoreboardDisplaySlot::TeamGold),
        "sidebar.team.gray" => Some(ScoreboardDisplaySlot::TeamGray),
        "sidebar.team.dark_gray" => Some(ScoreboardDisplaySlot::TeamDarkGray),
        "sidebar.team.blue" => Some(ScoreboardDisplaySlot::TeamBlue),
        "sidebar.team.green" => Some(ScoreboardDisplaySlot::TeamGreen),
        "sidebar.team.aqua" => Some(ScoreboardDisplaySlot::TeamAqua),
        "sidebar.team.red" => Some(ScoreboardDisplaySlot::TeamRed),
        "sidebar.team.light_purple" => Some(ScoreboardDisplaySlot::TeamLightPurple),
        "sidebar.team.yellow" => Some(ScoreboardDisplaySlot::TeamYellow),
        "sidebar.team.white" => Some(ScoreboardDisplaySlot::TeamWhite),
        _ => None,
    }
}

#[derive(Default)]
pub struct Scoreboard {
    objectives: HashMap<String, ScoreboardObjective>,
    teams: HashMap<String, Team>,
    scores: HashMap<String, HashMap<String, ScoreboardScore>>,
    /// Tracks which objective (by name) is currently displayed in each display slot.
    /// A `None` value means the slot is empty (no objective displayed).
    display_objectives: HashMap<ScoreboardDisplaySlot, Option<String>>,
    /// Only tracked objectives are sent to clients via packets. An objective becomes tracked when it is
    /// assigned to a display slot. Creating an objective does NOT make it tracked.
    tracked_objectives: HashSet<String>,
    /// Reverse index from criterion name -> objective names.
    /// Enables efficient updates of all objectives tracking a given auto-computed criterion (health, food, etc.).
    objectives_by_criterion: HashMap<String, Vec<String>>,
}

impl Scoreboard {
    #[must_use]
    pub const fn get_objectives(&self) -> &HashMap<String, ScoreboardObjective> {
        &self.objectives
    }

    #[must_use]
    pub const fn get_scores(&self) -> &HashMap<String, HashMap<String, ScoreboardScore>> {
        &self.scores
    }

    #[must_use]
    pub const fn get_teams(&self) -> &HashMap<String, Team> {
        &self.teams
    }

    async fn broadcast_editioned<J: ClientPacket, B: BClientPacket>(
        world: &World,
        je_packet: &J,
        be_packet: &B,
    ) {
        world.broadcast_editioned(je_packet, be_packet).await;
    }

    /// Adds an objective to the scoreboard without sending any packets.
    pub fn add_objective(&mut self, _world: &World, objective: ScoreboardObjective) {
        if self.objectives.contains_key(&*objective.name) {
            warn!(
                "Tried to create an objective which already exists: {}",
                &*objective.name
            );
            return;
        }

        let criterion = objective.criterion.to_string();
        self.objectives_by_criterion
            .entry(criterion)
            .or_default()
            .push(objective.name.to_string());
        self.objectives
            .insert(objective.name.to_string(), objective);
    }

    /// Updates display name, render type, number format, or display auto-update on an existing objective and
    /// sends the update packet to all players if the objective is tracked.
    pub fn modify_objective(
        &mut self,
        world: &World,
        objective_name: &str,
        display_name: TextComponent,
        render_type: RenderType,
        number_format: Option<NumberFormat>,
    ) -> bool {
        let Some(objective) = self.objectives.get_mut(objective_name) else {
            return false;
        };
        objective.display_name = display_name.clone();
        objective.render_type = render_type;
        objective.number_format.clone_from(&number_format);

        // Only send update packet if tracked
        if self.tracked_objectives.contains(objective_name) {
            let je_update = CUpdateObjectives::new(
                objective_name.to_string(),
                Mode::Update,
                display_name,
                render_type,
                number_format,
            );
            world.broadcast_packet_all(&je_update);
        }
        true
    }

    /// Sets the `display_auto_update` flag on an objective. When true, the score's
    /// display name is automatically updated to the score holder's display name
    /// whenever the score is modified.
    pub fn set_display_auto_update(
        &mut self,
        world: &World,
        objective_name: &str,
        display_auto_update: bool,
    ) -> bool {
        let Some(objective) = self.objectives.get_mut(objective_name) else {
            return false;
        };
        if objective.display_auto_update == display_auto_update {
            return true;
        }
        objective.display_auto_update = display_auto_update;

        // Sends update packet for tracked objectives
        if self.tracked_objectives.contains(objective_name) {
            let je_update = CUpdateObjectives::new(
                objective_name.to_string(),
                Mode::Update,
                objective.display_name.clone(),
                objective.render_type,
                objective.number_format.clone(),
            );
            world.broadcast_packet_all(&je_update);
        }
        true
    }

    pub fn set_objective_number_format(
        &mut self,
        world: &World,
        objective_name: &str,
        number_format: Option<NumberFormat>,
    ) -> bool {
        let Some(objective) = self.objectives.get_mut(objective_name) else {
            return false;
        };
        objective.number_format.clone_from(&number_format);

        if self.tracked_objectives.contains(objective_name) {
            let je_update = CUpdateObjectives::new(
                objective_name.to_string(),
                Mode::Update,
                objective.display_name.clone(),
                objective.render_type,
                number_format,
            );
            world.broadcast_packet_all(&je_update);
        }
        true
    }

    /// Sends the create-objective packet and all existing scores for this objective to all players.
    /// Called when an objective is assigned to a display slot for the first time.
    fn start_tracking_objective(&mut self, world: &World, objective_name: &str) {
        if self.tracked_objectives.contains(objective_name) {
            return; // already tracked
        }
        let Some(objective) = self.objectives.get(objective_name) else {
            return;
        };

        // Send CREATE objective packet
        let je_create = CUpdateObjectives::new(
            objective_name.to_string(),
            Mode::Add,
            objective.display_name.clone(),
            objective.render_type,
            objective.number_format.clone(),
        );
        world.broadcast_packet_all(&je_create);

        // Send all existing scores for this objective
        if let Some(objective_scores) = self.scores.get(objective_name) {
            for score in objective_scores.values() {
                let je_score = CUpdateScore::new(
                    score.entity_name.to_string(),
                    score.objective_name.to_string(),
                    score.value,
                    score.display_name.clone(),
                    score.number_format.clone(),
                );
                world.broadcast_packet_all(&je_score);
            }
        }

        self.tracked_objectives.insert(objective_name.to_string());
    }

    /// Sends the remove-objective packet and removes from tracking.
    fn stop_tracking_objective(&mut self, world: &World, objective_name: &str) {
        if !self.tracked_objectives.remove(objective_name) {
            return; // wasn't tracked
        }

        let je_remove = CUpdateObjectives::new(
            objective_name.to_string(),
            Mode::Remove,
            TextComponent::empty(),
            RenderType::Integer,
            None,
        );
        world.broadcast_packet_all(&je_remove);
    }

    pub fn remove_objective(&mut self, world: &World, name: &str) {
        if !self.objectives.contains_key(name) {
            warn!(
                "Tried to remove an objective which does not exist: {}",
                name
            );
            return;
        }

        // Stop tracking first (sends remove packet, clears display slots)
        self.stop_tracking_objective(world, name);

        // When removing an objective, clear any display slots showing it
        for (slot, displayed) in &self.display_objectives {
            if displayed.as_deref() == Some(name) {
                let clear_packet = CDisplayObjective::new(*slot, String::new());
                world.broadcast_packet_all(&clear_packet);
            }
        }
        self.display_objectives
            .retain(|_, v| v.as_deref() != Some(name));

        // Remove from objectives_by_criterion index
        if let Some(objective) = self.objectives.get(name) {
            let criterion = &*objective.criterion;
            if let Some(objectives) = self.objectives_by_criterion.get_mut(criterion) {
                objectives.retain(|n| n != name);
                if objectives.is_empty() {
                    self.objectives_by_criterion.remove(criterion);
                }
            }
        }

        self.objectives.remove(name);
        self.scores.remove(name);
    }

    /// Sets the objective to display in the given slot, or clears it if `None`.
    /// Broadcasts the display change to all players.
    pub fn set_display_objective(
        &mut self,
        world: &World,
        slot: ScoreboardDisplaySlot,
        objective_name: Option<&str>,
    ) {
        let old_objective = self.get_display_objective(&slot).map(ToString::to_string);
        let score_name = objective_name.unwrap_or("");

        if let Some(name) = objective_name {
            // If this objective is not yet tracked, start tracking it
            // (sends create packet + all scores + display packet)
            if !self.tracked_objectives.contains(name) {
                self.start_tracking_objective(world, name);
            }
        }

        let je_packet = CDisplayObjective::new(slot, score_name.to_string());
        world.broadcast_packet_all(&je_packet);
        self.display_objectives
            .insert(slot, objective_name.map(ToString::to_string));

        if let Some(old_name) = old_objective
            && objective_name != Some(old_name.as_str())
            && !self
                .display_objectives
                .values()
                .any(|displayed| displayed.as_deref() == Some(old_name.as_str()))
        {
            self.stop_tracking_objective(world, &old_name);
        }
    }

    /// Returns the name of the objective currently displayed in the given slot, or `None`.
    #[must_use]
    pub fn get_display_objective(&self, slot: &ScoreboardDisplaySlot) -> Option<&str> {
        self.display_objectives.get(slot).and_then(|v| v.as_deref())
    }

    #[must_use]
    pub fn get_objective(&self, name: &str) -> Option<&ScoreboardObjective> {
        self.objectives.get(name)
    }

    /// Updates all objectives that use the given criterion with the specified value for the entity.
    /// Automatically creates score entries if they don't exist.
    pub async fn for_all_objectives(
        &mut self,
        world: &World,
        criterion: &str,
        entity_name: &str,
        value: i32,
    ) {
        let Some(objective_names) = self.objectives_by_criterion.get(criterion).cloned() else {
            return;
        };

        for objective_name in &objective_names {
            let score = ScoreboardScore {
                entity_name: Arc::from(entity_name),
                objective_name: Arc::from(objective_name.as_str()),
                value: VarInt(value),
                display_name: None,
                number_format: None,
                locked: false,
            };
            self.update_score(world, score).await;
        }
    }

    /// Returns whether the given objective is currently tracked (displayed).
    #[must_use]
    pub fn is_tracked(&self, objective_name: &str) -> bool {
        self.tracked_objectives.contains(objective_name)
    }

    /// Returns all score holders that have at least one score entry.
    #[must_use]
    pub fn get_tracked_players(&self) -> Vec<&str> {
        let mut players: Vec<&str> = self
            .scores
            .values()
            .flat_map(|scores| scores.keys().map(String::as_str))
            .collect();
        players.sort_unstable();
        players.dedup();
        players
    }

    /// Resets a single score for a player. Sends a reset-score packet if the
    /// objective was tracked.
    pub fn reset_single_player_score(
        &mut self,
        world: &World,
        entity_name: &str,
        objective_name: &str,
    ) {
        let was_removed = self
            .scores
            .get_mut(objective_name)
            .is_some_and(|m| m.remove(entity_name).is_some());

        if was_removed && self.tracked_objectives.contains(objective_name) {
            let packet =
                CResetScore::new(entity_name.to_string(), Some(objective_name.to_string()));
            world.broadcast_packet_all(&packet);
        }
    }

    /// Resets all scores for a player across all objectives.
    pub fn reset_all_player_scores(&mut self, world: &World, entity_name: &str) {
        let mut any_removed = false;
        self.scores.retain(|objective_name, scores| {
            if scores.remove(entity_name).is_some() {
                any_removed = true;
                // Only send reset packets for tracked objectives
                if self.tracked_objectives.contains(objective_name) {
                    let packet =
                        CResetScore::new(entity_name.to_string(), Some(objective_name.clone()));
                    world.broadcast_packet_all(&packet);
                }
            }
            !scores.is_empty()
        });

        // Send a final reset with null objective if any scores were removed
        if any_removed {
            let packet = CResetScore::new(entity_name.to_string(), None);
            world.broadcast_packet_all(&packet);
        }
    }

    #[must_use]
    pub fn get_player_score_info(
        &self,
        entity_name: &str,
        objective_name: &str,
    ) -> Option<&ScoreboardScore> {
        self.scores
            .get(objective_name)
            .and_then(|m| m.get(entity_name))
    }

    pub fn set_score_number_format(
        &mut self,
        world: &World,
        entity_name: &str,
        objective_name: &str,
        number_format: Option<NumberFormat>,
    ) -> bool {
        let Some(score) = self
            .scores
            .get_mut(objective_name)
            .and_then(|m| m.get_mut(entity_name))
        else {
            return false;
        };
        score.number_format.clone_from(&number_format);
        if self.tracked_objectives.contains(objective_name) {
            let packet = CUpdateScore::new(
                entity_name.to_string(),
                objective_name.to_string(),
                score.value,
                score.display_name.clone(),
                number_format,
            );
            world.broadcast_packet_all(&packet);
        }
        true
    }

    /// Sets the optional display name for a score entry. Sends packet if tracked.
    pub fn set_score_display_name(
        &mut self,
        world: &World,
        entity_name: &str,
        objective_name: &str,
        display_name: Option<TextComponent>,
    ) -> bool {
        let Some(score) = self
            .scores
            .get_mut(objective_name)
            .and_then(|m| m.get_mut(entity_name))
        else {
            return false;
        };
        score.display_name.clone_from(&display_name);
        if self.tracked_objectives.contains(objective_name) {
            let packet = CUpdateScore::new(
                entity_name.to_string(),
                objective_name.to_string(),
                score.value,
                display_name,
                score.number_format.clone(),
            );
            world.broadcast_packet_all(&packet);
        }
        true
    }

    #[must_use]
    pub fn list_scores_for_objective(&self, objective_name: &str) -> Vec<&ScoreboardScore> {
        self.scores
            .get(objective_name)
            .map(|m| m.values().collect())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn list_scores_for_player(&self, entity_name: &str) -> Vec<(&str, i32)> {
        let mut result = Vec::new();
        for (obj_name, scores) in &self.scores {
            if let Some(score) = scores.get(entity_name) {
                result.push((obj_name.as_str(), score.value.0));
            }
        }
        result
    }

    /// Sends the full scoreboard state (all tracked objectives + display slots + scores)
    /// to a single player. Called when a player joins the world.
    pub fn send_state_to_player(&self, player: &crate::entity::player::Player) {
        // Teams are global scoreboard state too, and must be created client-side
        // before later membership or update packets can refer to them.
        for team in self.teams.values() {
            let parameters = TeamParameters {
                display_name: &team.display_name,
                options: team.options,
                nametag_visibility: team.nametag_visibility.to_str(),
                collision_rule: team.collision_rule.to_str(),
                color: team.color as i32,
                player_prefix: &team.player_prefix,
                player_suffix: &team.player_suffix,
            };
            let packet = CSetPlayerTeam {
                team_name: team.name.clone(),
                method: TeamMethod::Create,
                parameters: Some(parameters),
                players: team.players.clone().into(),
            };
            player.client.try_enqueue_packet(&packet);
        }

        // Send all tracked objectives (create packet + scores)
        for objective_name in &self.tracked_objectives {
            let Some(objective) = self.objectives.get(objective_name.as_str()) else {
                continue;
            };
            let je_create = CUpdateObjectives::new(
                objective_name.clone(),
                Mode::Add,
                objective.display_name.clone(),
                objective.render_type,
                objective.number_format.clone(),
            );
            // FIXME: using try_enqueue_packet since this is called in spawn context
            player.client.try_enqueue_packet(&je_create);

            // Send all scores for this objective
            if let Some(objective_scores) = self.scores.get(objective_name.as_str()) {
                for score in objective_scores.values() {
                    let je_score = CUpdateScore::new(
                        score.entity_name.to_string(),
                        score.objective_name.to_string(),
                        score.value,
                        score.display_name.clone(),
                        score.number_format.clone(),
                    );
                    player.client.try_enqueue_packet(&je_score);
                }
            }
        }

        // Send display slot bindings
        for (slot, objective_name) in &self.display_objectives {
            let name = objective_name.as_deref().unwrap_or("");
            let je_display = CDisplayObjective::new(*slot, name.to_string());
            player.client.try_enqueue_packet(&je_display);
        }
    }

    /// Converts the in-memory scoreboard to a serializable data structure for disk storage.
    #[must_use]
    pub fn to_data(&self) -> pumpkin_world::world_info::data_files::ScoreboardData {
        use pumpkin_protocol::java::client::play::RenderType;
        use pumpkin_world::world_info::data_files::{
            SerializableNumberFormat, SerializableObjective, SerializableScore, SerializableTeam,
            SerializableTextComponent,
        };

        fn serialize_number_format(format: &NumberFormat) -> SerializableNumberFormat {
            match format {
                NumberFormat::Blank => SerializableNumberFormat::Blank,
                NumberFormat::Styled(style) => SerializableNumberFormat::Styled {
                    style: style.clone(),
                },
                NumberFormat::Fixed(value) => SerializableNumberFormat::Fixed {
                    value: SerializableTextComponent(value.clone()),
                },
            }
        }

        let objectives = self
            .objectives
            .values()
            .map(|obj| SerializableObjective {
                name: obj.name.to_string(),
                display_name: SerializableTextComponent(obj.display_name.clone()),
                render_type: match obj.render_type {
                    RenderType::Integer => "integer".to_string(),
                    RenderType::Hearts => "hearts".to_string(),
                },
                criteria_name: obj.criterion.to_string(),
                display_auto_update: obj.display_auto_update,
                number_format: obj.number_format.as_ref().map(serialize_number_format),
            })
            .collect();

        let mut scores = Vec::new();
        for (obj_name, obj_scores) in &self.scores {
            for score in obj_scores.values() {
                scores.push(SerializableScore {
                    entity_name: score.entity_name.to_string(),
                    objective_name: obj_name.clone(),
                    value: score.value.0,
                    locked: score.locked,
                    display: score.display_name.clone().map(SerializableTextComponent),
                    number_format: score.number_format.as_ref().map(serialize_number_format),
                });
            }
        }

        let teams = self
            .teams
            .values()
            .map(|t| SerializableTeam {
                name: t.name.clone(),
                display_name: SerializableTextComponent(t.display_name.clone()),
                color: t.color,
                allow_friendly_fire: (t.options & 0x01) != 0,
                see_friendly_invisibles: (t.options & 0x02) != 0,
                player_prefix: SerializableTextComponent(t.player_prefix.clone()),
                player_suffix: SerializableTextComponent(t.player_suffix.clone()),
                nametag_visibility: t.nametag_visibility.to_str().to_string(),
                death_message_visibility: t.death_message_visibility.to_str().to_string(),
                collision_rule: t.collision_rule.to_str().to_string(),
                players: t.players.clone(),
            })
            .collect();

        let display_slots = self
            .display_objectives
            .iter()
            .filter_map(|(slot, name)| {
                name.as_ref()
                    .map(|n| (display_slot_name(*slot).to_string(), n.clone()))
            })
            .collect();

        pumpkin_world::world_info::data_files::ScoreboardData {
            objectives,
            scores,
            teams,
            display_slots,
        }
    }

    /// Populates this scoreboard from serialized data (loaded from disk).
    pub fn load_from_data(&mut self, data: &pumpkin_world::world_info::data_files::ScoreboardData) {
        use pumpkin_protocol::java::client::play::RenderType;
        use pumpkin_world::world_info::data_files::SerializableNumberFormat;

        fn deserialize_number_format(format: &SerializableNumberFormat) -> NumberFormat {
            match format {
                SerializableNumberFormat::Blank => NumberFormat::Blank,
                SerializableNumberFormat::Styled { style } => NumberFormat::Styled(style.clone()),
                SerializableNumberFormat::Fixed { value } => NumberFormat::Fixed(value.0.clone()),
            }
        }

        // Clear existing data
        self.objectives.clear();
        self.teams.clear();
        self.scores.clear();
        self.display_objectives.clear();
        self.tracked_objectives.clear();
        self.objectives_by_criterion.clear();

        // Load objectives
        for obj in &data.objectives {
            let render_type = if obj.render_type == "hearts" {
                RenderType::Hearts
            } else {
                RenderType::Integer
            };
            let number_format = obj.number_format.as_ref().map(deserialize_number_format);
            let mut objective = ScoreboardObjective::new(
                Arc::from(obj.name.as_str()),
                obj.display_name.0.clone(),
                render_type,
                number_format,
                Arc::from(obj.criteria_name.as_str()),
            );
            objective.display_auto_update = obj.display_auto_update;
            let criterion = obj.criteria_name.clone();
            self.objectives_by_criterion
                .entry(criterion)
                .or_default()
                .push(obj.name.clone());
            self.objectives.insert(obj.name.clone(), objective);
        }

        // Load scores
        for score in &data.scores {
            let number_format = score.number_format.as_ref().map(deserialize_number_format);
            let entry = self.scores.entry(score.objective_name.clone()).or_default();
            entry.insert(
                score.entity_name.clone(),
                ScoreboardScore {
                    entity_name: Arc::from(score.entity_name.as_str()),
                    objective_name: Arc::from(score.objective_name.as_str()),
                    value: VarInt(score.value),
                    display_name: score.display.as_ref().map(|display| display.0.clone()),
                    number_format,
                    locked: score.locked,
                },
            );
        }

        // Load teams
        for team in &data.teams {
            let mut options = 0;
            if team.allow_friendly_fire {
                options |= 0x01;
            }
            if team.see_friendly_invisibles {
                options |= 0x02;
            }
            self.teams.insert(
                team.name.clone(),
                Team {
                    name: team.name.clone(),
                    display_name: team.display_name.0.clone(),
                    options,
                    nametag_visibility: NameTagVisibility::from_name(&team.nametag_visibility),
                    death_message_visibility: NameTagVisibility::from_name(
                        &team.death_message_visibility,
                    ),
                    collision_rule: CollisionRule::from_name(&team.collision_rule),
                    color: team.color,
                    player_prefix: team.player_prefix.0.clone(),
                    player_suffix: team.player_suffix.0.clone(),
                    players: team.players.clone(),
                },
            );
        }

        // Load display slots (stored as string keys in NBT)
        for (slot_name, obj_name) in &data.display_slots {
            if let Some(slot) = display_slot_from_name(slot_name) {
                self.display_objectives.insert(slot, Some(obj_name.clone()));
            }
        }

        // Repopulate tracked_objectives: any objective assigned to a display slot is "tracked"
        for name in self.display_objectives.values().flatten() {
            self.tracked_objectives.insert(name.clone());
        }
    }

    pub async fn update_score(&mut self, world: &World, mut score: ScoreboardScore) {
        if !self.objectives.contains_key(&*score.objective_name) {
            warn!(
                "Tried to place a score into an objective which does not exist: {}",
                &score.objective_name
            );
            return;
        }

        // If the objective has this flag set, the score's display name is auto-updated to the score holder's display name.
        if let Some(objective) = self.objectives.get(&*score.objective_name)
            && objective.display_auto_update
        {
            let entity_display = TextComponent::text(score.entity_name.to_string());
            let should_update = score.display_name.as_ref() != Some(&entity_display);
            if should_update {
                score.display_name = Some(entity_display);
            }
        }

        // Only send packets for tracked objectives
        if self.tracked_objectives.contains(&*score.objective_name) {
            let je_packet = CUpdateScore::new(
                score.entity_name.to_string(),
                score.objective_name.to_string(),
                score.value,
                score.display_name.clone(),
                score.number_format.clone(),
            );

            let be_packet = BSetScore {
                action: VarInt(0), // Change
                entries: vec![BScoreEntry {
                    scoreboard_id: score.entity_name.as_ptr() as i64, // Hacky ID
                    objective_name: score.objective_name.to_string(),
                    score: score.value,
                    entry_type: VarInt(3), // Fake player/Literal
                    entity_unique_id: 0,
                    custom_name: score.entity_name.to_string(),
                }],
            };

            Self::broadcast_editioned(world, &je_packet, &be_packet).await;
        }

        self.scores
            .entry(score.objective_name.to_string())
            .or_default()
            .insert(score.entity_name.to_string(), score);
    }

    /// Removes a score. Only sends packet if the objective is tracked.
    pub async fn remove_score(&mut self, world: &World, entity_name: &str, objective_name: &str) {
        // Only send remove packet if objective is tracked
        if self.tracked_objectives.contains(objective_name) {
            let je_packet =
                CUpdateScore::new_remove(entity_name.to_string(), objective_name.to_string());

            let be_packet = BSetScore {
                action: VarInt(1), // Remove
                entries: vec![BScoreEntry {
                    scoreboard_id: entity_name.as_ptr() as i64,
                    objective_name: objective_name.to_string(),
                    score: VarInt(0),
                    entry_type: VarInt(3),
                    entity_unique_id: 0,
                    custom_name: entity_name.to_string(),
                }],
            };

            Self::broadcast_editioned(world, &je_packet, &be_packet).await;
        }

        if let Some(objective_scores) = self.scores.get_mut(objective_name) {
            objective_scores.remove(entity_name);
        }
    }

    pub fn add_team(&mut self, world: &World, team: Team) {
        if self.teams.contains_key(&team.name) {
            warn!(
                "Tried to create Team which does already exist, {}",
                team.name
            );
            return;
        }

        let parameters = TeamParameters {
            display_name: &team.display_name,
            options: team.options,
            nametag_visibility: team.nametag_visibility.to_str(),
            collision_rule: team.collision_rule.to_str(),
            color: team.color as i32,
            player_prefix: &team.player_prefix,
            player_suffix: &team.player_suffix,
        };

        world.broadcast_packet_all(&CSetPlayerTeam {
            team_name: team.name.clone(),
            method: TeamMethod::Create,
            parameters: Some(parameters),
            players: team.players.clone().into(),
        });

        self.teams.insert(team.name.clone(), team);
    }

    pub fn update_team(&mut self, world: &World, team: Team) {
        if !self.teams.contains_key(&team.name) {
            warn!("Tried to update Team which does not exist, {}", team.name);
            return;
        }

        let parameters = TeamParameters {
            display_name: &team.display_name,
            options: team.options,
            nametag_visibility: team.nametag_visibility.to_str(),
            collision_rule: team.collision_rule.to_str(),
            color: team.color as i32,
            player_prefix: &team.player_prefix,
            player_suffix: &team.player_suffix,
        };

        world.broadcast_packet_all(&CSetPlayerTeam {
            team_name: team.name.clone(),
            method: TeamMethod::Update,
            parameters: Some(parameters),
            players: Box::new([]),
        });

        self.teams.insert(team.name.clone(), team);
    }

    pub fn remove_team(&mut self, world: &World, name: &str) {
        if !self.teams.contains_key(name) {
            warn!("Tried to remove Team which does not exist, {}", name);
            return;
        }

        world.broadcast_packet_all(&CSetPlayerTeam {
            team_name: name.to_string(),
            method: TeamMethod::Remove,
            parameters: None,
            players: Box::new([]),
        });

        self.teams.remove(name);
    }

    pub fn add_player_to_team(&mut self, world: &World, team_name: &str, player: String) {
        if !self.teams.contains_key(team_name) {
            warn!(
                "Tried to add player to Team which does not exist, {}",
                team_name
            );
            return;
        }

        let previous_teams: Vec<String> = self
            .teams
            .iter()
            .filter(|(name, team)| *name != team_name && team.players.contains(&player))
            .map(|(name, _)| name.clone())
            .collect();
        for previous_team in previous_teams {
            self.remove_player_from_team(world, &previous_team, &player);
        }

        let team = self
            .teams
            .get_mut(team_name)
            .expect("team existence was checked");

        if team.players.contains(&player) {
            return;
        }

        world.broadcast_packet_all(&CSetPlayerTeam {
            team_name: team_name.to_string(),
            method: TeamMethod::AddPlayers,
            parameters: None,
            players: vec![player.clone()].into(),
        });

        team.players.push(player);
    }

    pub fn remove_player_from_team(&mut self, world: &World, team_name: &str, player: &str) {
        let Some(team) = self.teams.get_mut(team_name) else {
            warn!(
                "Tried to remove player from Team which does not exist, {}",
                team_name
            );
            return;
        };

        if !team.players.contains(&player.to_string()) {
            return;
        }

        world.broadcast_packet_all(&CSetPlayerTeam {
            team_name: team_name.to_string(),
            method: TeamMethod::RemovePlayers,
            parameters: None,
            players: vec![player.to_string()].into(),
        });

        team.players.retain(|p| p != player);
    }
}

pub struct ScoreboardObjective {
    pub name: Arc<str>,
    pub display_name: TextComponent,
    pub render_type: RenderType,
    pub number_format: Option<NumberFormat>,
    pub criterion: Arc<str>,
    pub display_auto_update: bool,
}

impl ScoreboardObjective {
    #[must_use]
    pub const fn new(
        name: Arc<str>,
        display_name: TextComponent,
        render_type: RenderType,
        number_format: Option<NumberFormat>,
        criterion: Arc<str>,
    ) -> Self {
        Self {
            name,
            display_name,
            render_type,
            number_format,
            criterion,
            display_auto_update: false,
        }
    }
}

#[derive(Clone)]
pub struct ScoreboardScore {
    pub entity_name: Arc<str>,
    pub objective_name: Arc<str>,
    pub value: VarInt,
    pub display_name: Option<TextComponent>,
    pub number_format: Option<NumberFormat>,
    pub locked: bool,
}

impl ScoreboardScore {
    #[must_use]
    pub const fn new(
        entity_name: Arc<str>,
        objective_name: Arc<str>,
        value: VarInt,
        display_name: Option<TextComponent>,
        number_format: Option<NumberFormat>,
    ) -> Self {
        Self {
            entity_name,
            objective_name,
            value,
            display_name,
            number_format,
            locked: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameTagVisibility {
    Always,
    Never,
    HideForOtherTeams,
    HideForOwnTeam,
}

impl NameTagVisibility {
    #[must_use]
    pub const fn to_str(&self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
            Self::HideForOtherTeams => "hideForOtherTeams",
            Self::HideForOwnTeam => "hideForOwnTeam",
        }
    }

    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "never" => Self::Never,
            "hideForOtherTeams" => Self::HideForOtherTeams,
            "hideForOwnTeam" => Self::HideForOwnTeam,
            _ => Self::Always,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionRule {
    Always,
    Never,
    PushOtherTeams,
    PushOwnTeam,
}

impl CollisionRule {
    #[must_use]
    pub const fn to_str(&self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
            Self::PushOtherTeams => "pushOtherTeams",
            Self::PushOwnTeam => "pushOwnTeam",
        }
    }

    #[must_use]
    pub fn from_name(value: &str) -> Self {
        match value {
            "never" => Self::Never,
            "pushOtherTeams" => Self::PushOtherTeams,
            "pushOwnTeam" => Self::PushOwnTeam,
            _ => Self::Always,
        }
    }
}

#[derive(Clone)]
pub struct Team {
    pub name: String,
    pub display_name: TextComponent,
    pub options: i8,
    pub nametag_visibility: NameTagVisibility,
    pub death_message_visibility: NameTagVisibility,
    pub collision_rule: CollisionRule,
    pub color: NamedColor,
    pub player_prefix: TextComponent,
    pub player_suffix: TextComponent,
    pub players: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{is_valid_criterion, statistic_criterion};
    use pumpkin_data::statistic::{CustomStatistic, StatisticCategory};

    #[test]
    fn accepts_modern_statistic_criteria() {
        assert!(is_valid_criterion("minecraft.custom:minecraft.sneak_time"));
        assert!(is_valid_criterion("minecraft.custom:minecraft.boat_one_cm"));
        assert!(is_valid_criterion(
            "minecraft.custom:minecraft.interact_with_anvil"
        ));
        assert!(!is_valid_criterion("stat.custom.sneakTime"));
        assert!(!is_valid_criterion("minecraft.custom:"));
    }

    #[test]
    fn maps_custom_statistics_to_vanilla_criteria() {
        assert_eq!(
            statistic_criterion(
                StatisticCategory::Custom,
                CustomStatistic::InteractWithAnvil as i32,
            )
            .as_deref(),
            Some("minecraft.custom:minecraft.interact_with_anvil")
        );
    }
}
