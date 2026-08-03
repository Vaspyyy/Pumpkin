use crate::codec::item_stack_seralizer::ItemStackTemplateSerializer;
use crate::codec::var_int::VarInt;
use pumpkin_data::Advancement;
use pumpkin_data::advancement_data::AdvancementProgressData;
use pumpkin_data::advancement_data::FrameType;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::packet::clientbound::PLAY_UPDATE_ADVANCEMENTS;
use pumpkin_macros::java_packet;
use pumpkin_util::identifier::Identifier;
use pumpkin_util::text::TextComponent;

use crate::ClientPacket;
use crate::ser::NetworkWriteExt;
use pumpkin_util::version::JavaMinecraftVersion;

#[java_packet(PLAY_UPDATE_ADVANCEMENTS)]
#[allow(unused)]
pub struct CUpdateAdvancements {
    pub reset: bool,
    pub added: Vec<ClientAdvancement>,
    pub removed: Vec<Identifier>,
    pub progress: Vec<AdvancementProgressData>,
    pub show_advancements: bool,
}

#[derive(Clone)]
pub struct ClientAdvancementDisplay {
    pub title: TextComponent,
    pub description: TextComponent,
    pub item_icon: ItemStack,
    pub frame_type: FrameType,
    pub background_texture: Option<String>,
    pub show_toast: bool,
    pub hidden: bool,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone)]
pub struct ClientAdvancement {
    pub id: Identifier,
    pub parent: Option<Identifier>,
    pub display: Option<ClientAdvancementDisplay>,
    pub requirements: Vec<Vec<String>>,
    pub send_telemetry: bool,
}

impl From<&'static Advancement> for ClientAdvancement {
    fn from(advancement: &'static Advancement) -> Self {
        Self {
            id: advancement.id.clone(),
            parent: advancement.parent.clone(),
            display: advancement.display.map(|display| ClientAdvancementDisplay {
                title: display.get_title(),
                description: display.get_description(),
                item_icon: display.item_icon.clone(),
                frame_type: display.frame_type,
                background_texture: display.background_texture.map(str::to_owned),
                show_toast: display.show_toast,
                hidden: display.hidden,
                x: display.x,
                y: display.y,
            }),
            requirements: advancement
                .requirements
                .iter()
                .map(|requirement| {
                    requirement
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect()
                })
                .collect(),
            send_telemetry: advancement.send_telemetry,
        }
    }
}

impl CUpdateAdvancements {
    #[must_use]
    #[allow(unused)]
    pub const fn new(
        reset: bool,
        added: Vec<ClientAdvancement>,
        progress: Vec<AdvancementProgressData>,
        removed: Vec<Identifier>,
        show_advancements: bool,
    ) -> Self {
        Self {
            reset,
            added,
            removed,
            progress,
            show_advancements,
        }
    }
}

impl ClientPacket for CUpdateAdvancements {
    #[allow(clippy::unimplemented)]
    fn write_packet_data(
        &self,
        mut write: impl std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), crate::ser::WritingError> {
        write.write_bool(self.reset)?;

        write.write_var_int(&VarInt(self.added.len() as i32))?;
        for adv in &self.added {
            write.write_string(&adv.id.to_string())?;

            let has_parent = adv.parent.is_some();
            write.write_bool(has_parent)?;
            if let Some(ref p) = adv.parent {
                write.write_string(&p.to_string())?;
            }

            let has_display = adv.display.is_some();
            write.write_bool(has_display)?;
            if let Some(display) = &adv.display {
                write.write_slice(&display.title.clone().encode())?;
                write.write_slice(&display.description.clone().encode())?;

                // Item icon
                ItemStackTemplateSerializer::from(display.item_icon.clone())
                    .write_with_version(&mut write, version)?;

                write.write_var_int(&VarInt(display.frame_type as i32))?;
                let flags = (display.background_texture.is_some() as i32)
                    | ((display.show_toast as i32) << 1)
                    | ((display.hidden as i32) << 2);
                write.write_i32_be(flags)?;
                if let Some(bg) = &display.background_texture {
                    write.write_string(bg)?;
                }
                write.write_f32_be(display.x)?;
                write.write_f32_be(display.y)?;
            }

            write.write_var_int(&VarInt(adv.requirements.len() as i32))?;
            for req in &adv.requirements {
                write.write_var_int(&VarInt(req.len() as i32))?;
                for r in req {
                    write.write_string(r)?;
                }
            }

            write.write_bool(adv.send_telemetry)?;
        }

        write.write_var_int(&VarInt(self.removed.len() as i32))?;
        for rem in &self.removed {
            write.write_string(&rem.to_string())?;
        }

        write.write_var_int(&VarInt(self.progress.len() as i32))?;
        for prog in &self.progress {
            write.write_string(&prog.id.to_string())?;
            write.write_var_int(&VarInt(prog.progress.len() as i32))?;
            for crit in &prog.progress {
                write.write_string(&crit.criterion_id)?;
                let has_date = crit.achieve_date.is_some();
                write.write_bool(has_date)?;
                if let Some(date) = crit.achieve_date {
                    write.write_i64_be(date)?;
                }
            }
        }

        write.write_bool(self.show_advancements)?;

        Ok(())
    }
}
