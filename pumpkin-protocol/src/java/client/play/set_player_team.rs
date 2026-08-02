use std::io::Write;

use pumpkin_data::packet::clientbound::PLAY_SET_PLAYER_TEAM;
use pumpkin_macros::java_packet;
use pumpkin_util::{text::TextComponent, version::JavaMinecraftVersion};

use crate::{
    ClientPacket,
    codec::var_int::VarInt,
    ser::{NetworkWriteExt, WritingError},
};

#[repr(i8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TeamMethod {
    Create = 0,
    Remove = 1,
    Update = 2,
    AddPlayers = 3,
    RemovePlayers = 4,
}

pub struct TeamParameters<'a> {
    pub display_name: &'a TextComponent,
    pub options: i8,
    pub nametag_visibility: &'a str,
    pub collision_rule: &'a str,
    pub color: i32,
    pub player_prefix: &'a TextComponent,
    pub player_suffix: &'a TextComponent,
}

#[java_packet(PLAY_SET_PLAYER_TEAM)]
pub struct CSetPlayerTeam<'a> {
    pub team_name: String,
    pub method: TeamMethod,
    pub parameters: Option<TeamParameters<'a>>,
    pub players: Box<[String]>,
}

impl ClientPacket for CSetPlayerTeam<'_> {
    fn write_packet_data(
        &self,
        mut write: impl Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), WritingError> {
        write.write_string(&self.team_name)?;
        write.write_i8(self.method as i8)?;

        match self.method {
            TeamMethod::Create | TeamMethod::Update => {
                if let Some(params) = &self.parameters {
                    write.write_slice(&params.display_name.encode())?;
                    if version >= &JavaMinecraftVersion::V_26_2 {
                        // 26.2 replaced the old ad-hoc parameter encoding with a
                        // composite stream codec and reordered most of its fields.
                        write.write_slice(&params.player_prefix.encode())?;
                        write.write_slice(&params.player_suffix.encode())?;
                        write.write_var_int(&VarInt(visibility_id(params.nametag_visibility)))?;
                        write.write_var_int(&VarInt(collision_rule_id(params.collision_rule)))?;
                        write.write_bool(true)?;
                        write.write_var_int(&VarInt(params.color))?;
                        write.write_i8(params.options)?;
                    } else {
                        write.write_i8(params.options)?;
                        if version >= &JavaMinecraftVersion::V_26_1 {
                            write
                                .write_var_int(&VarInt(visibility_id(params.nametag_visibility)))?;
                            write
                                .write_var_int(&VarInt(collision_rule_id(params.collision_rule)))?;
                        } else {
                            write.write_string(params.nametag_visibility)?;
                            write.write_string(params.collision_rule)?;
                        }
                        write.write_var_int(&VarInt(params.color))?;
                        write.write_slice(&params.player_prefix.encode())?;
                        write.write_slice(&params.player_suffix.encode())?;
                    }
                } else {
                    return Err(WritingError::Message(
                        "Parameters missing for Create/Update".into(),
                    ));
                }
            }
            _ => {}
        }

        match self.method {
            TeamMethod::Create | TeamMethod::AddPlayers | TeamMethod::RemovePlayers => {
                write.write_var_int(&VarInt(self.players.len() as i32))?;
                for player in &self.players {
                    write.write_string(player)?;
                }
            }
            _ => {}
        }

        Ok(())
    }
}

const fn visibility_id(visibility: &str) -> i32 {
    match visibility.as_bytes() {
        b"never" => 1,
        b"hideForOtherTeams" => 2,
        b"hideForOwnTeam" => 3,
        _ => 0,
    }
}

const fn collision_rule_id(rule: &str) -> i32 {
    match rule.as_bytes() {
        b"never" => 1,
        b"pushOtherTeams" => 2,
        b"pushOwnTeam" => 3,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{CSetPlayerTeam, TeamMethod, TeamParameters};
    use crate::{ClientPacket, codec::var_int::VarInt, ser::NetworkWriteExt};
    use pumpkin_util::{text::TextComponent, version::JavaMinecraftVersion};

    fn test_packet<'a>(
        display_name: &'a TextComponent,
        prefix: &'a TextComponent,
        suffix: &'a TextComponent,
    ) -> CSetPlayerTeam<'a> {
        CSetPlayerTeam {
            team_name: "codex_team".to_string(),
            method: TeamMethod::Create,
            parameters: Some(TeamParameters {
                display_name,
                options: 3,
                nametag_visibility: "hideForOwnTeam",
                collision_rule: "pushOtherTeams",
                color: 15,
                player_prefix: prefix,
                player_suffix: suffix,
            }),
            players: vec!["Vaspyyy".to_string()].into_boxed_slice(),
        }
    }

    #[test]
    fn encodes_26_2_team_parameters_in_stream_codec_order() {
        let display_name = TextComponent::text("Codex Team");
        let prefix = TextComponent::text("[");
        let suffix = TextComponent::text("]");
        let packet = test_packet(&display_name, &prefix, &suffix);
        let mut actual = Vec::new();
        packet
            .write_packet_data(&mut actual, &JavaMinecraftVersion::V_26_2)
            .unwrap();

        let mut expected = Vec::new();
        expected.write_string("codex_team").unwrap();
        expected.write_i8(TeamMethod::Create as i8).unwrap();
        expected.write_slice(&display_name.encode()).unwrap();
        expected.write_slice(&prefix.encode()).unwrap();
        expected.write_slice(&suffix.encode()).unwrap();
        expected.write_var_int(&VarInt(3)).unwrap();
        expected.write_var_int(&VarInt(2)).unwrap();
        expected.write_bool(true).unwrap();
        expected.write_var_int(&VarInt(15)).unwrap();
        expected.write_i8(3).unwrap();
        expected.write_var_int(&VarInt(1)).unwrap();
        expected.write_string("Vaspyyy").unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn encodes_26_1_enum_ids_without_26_2_reordering() {
        let display_name = TextComponent::text("Codex Team");
        let prefix = TextComponent::text("[");
        let suffix = TextComponent::text("]");
        let packet = test_packet(&display_name, &prefix, &suffix);
        let mut actual = Vec::new();
        packet
            .write_packet_data(&mut actual, &JavaMinecraftVersion::V_26_1)
            .unwrap();

        let mut expected = Vec::new();
        expected.write_string("codex_team").unwrap();
        expected.write_i8(TeamMethod::Create as i8).unwrap();
        expected.write_slice(&display_name.encode()).unwrap();
        expected.write_i8(3).unwrap();
        expected.write_var_int(&VarInt(3)).unwrap();
        expected.write_var_int(&VarInt(2)).unwrap();
        expected.write_var_int(&VarInt(15)).unwrap();
        expected.write_slice(&prefix.encode()).unwrap();
        expected.write_slice(&suffix.encode()).unwrap();
        expected.write_var_int(&VarInt(1)).unwrap();
        expected.write_string("Vaspyyy").unwrap();

        assert_eq!(actual, expected);
    }
}
