use crate::command::CommandSender;
use crate::command::context::command_source::CommandSource;
use crate::command::node::dispatcher::UnifiedCommandError;
use crate::server::Server;
use pumpkin_data::biome::Biome;
use pumpkin_data::entity::EntityType;
use pumpkin_data::tag::{RegistryKey, get_registry_key_tags};
use pumpkin_util::identifier::Identifier;
use pumpkin_util::version::JavaMinecraftVersion;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;
use std::cell::Cell;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};
use zip::ZipArchive;

const MAX_DATA_PACK_FILE_SIZE: u64 = 4 * 1024 * 1024;
const MAX_FUNCTION_DEPTH: usize = 128;
const CURRENT_DATA_PACK_FORMAT: f64 = 107.1;

tokio::task_local! {
    static FUNCTION_EXECUTION_STATE: FunctionExecutionState;
}

struct FunctionExecutionState {
    commands: Cell<i64>,
    depth: Cell<usize>,
    max_commands: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum FunctionExecutionError {
    #[error("function {0} does not exist")]
    UnknownFunction(Identifier),
    #[error("function tag {0} does not exist")]
    UnknownTag(Identifier),
    #[error("function recursion exceeded the maximum depth of {MAX_FUNCTION_DEPTH}")]
    RecursionLimit,
    #[error("function command chain exceeded max_command_sequence_length ({0})")]
    CommandLimit(i64),
}

#[derive(Debug, Clone)]
pub struct FunctionCommand {
    pub line: usize,
    pub command: Arc<str>,
}

#[derive(Debug, Clone)]
pub struct DataPackFunction {
    pub id: Identifier,
    pub commands: Arc<[FunctionCommand]>,
}

#[derive(Debug)]
pub struct DataPackManager {
    functions: FxHashMap<Identifier, Arc<DataPackFunction>>,
    tags: FxHashMap<Identifier, Vec<TagEntry>>,
    entity_type_tags: FxHashMap<Identifier, Vec<EntityTypeTagEntry>>,
    biome_tags: FxHashMap<Identifier, Vec<BiomeTagEntry>>,
    load_functions: Arc<[Identifier]>,
    tick_functions: Arc<[Identifier]>,
    loaded_packs: Arc<[String]>,
    reported_command_errors: Mutex<FxHashSet<String>>,
}

impl Default for DataPackManager {
    fn default() -> Self {
        Self {
            functions: FxHashMap::default(),
            tags: FxHashMap::default(),
            entity_type_tags: vanilla_entity_type_tags(),
            biome_tags: vanilla_biome_tags(),
            load_functions: Arc::from([]),
            tick_functions: Arc::from([]),
            loaded_packs: Arc::from([]),
            reported_command_errors: Mutex::new(FxHashSet::default()),
        }
    }
}

#[derive(Debug, Clone)]
struct TagEntry {
    id: Identifier,
    is_tag: bool,
    required: bool,
}

#[derive(Debug, Clone)]
enum EntityTypeTagEntry {
    EntityType(u16),
    Tag { id: Identifier, required: bool },
}

#[derive(Debug, Clone)]
enum BiomeTagEntry {
    Biome(u8),
    Tag { id: Identifier, required: bool },
}

#[derive(Debug)]
struct PackFile {
    path: String,
    contents: Vec<u8>,
}

#[derive(Debug)]
struct LoadedPack {
    name: String,
    files: Vec<PackFile>,
}

#[derive(Debug, Deserialize)]
struct TagFile {
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    values: Vec<TagFileValue>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TagFileValue {
    Id(String),
    Detailed {
        id: String,
        #[serde(default = "required_by_default")]
        required: bool,
    },
}

const fn required_by_default() -> bool {
    true
}

fn vanilla_entity_type_tags() -> FxHashMap<Identifier, Vec<EntityTypeTagEntry>> {
    let mut tags = FxHashMap::default();
    let Some(vanilla_tags) =
        get_registry_key_tags(JavaMinecraftVersion::V_26_2, RegistryKey::EntityType)
    else {
        return tags;
    };
    for (raw_id, values) in vanilla_tags.entries() {
        let Ok(id) = Identifier::parse(raw_id) else {
            continue;
        };
        tags.insert(
            id,
            values
                .1
                .iter()
                .copied()
                .map(EntityTypeTagEntry::EntityType)
                .collect(),
        );
    }
    tags
}

fn vanilla_biome_tags() -> FxHashMap<Identifier, Vec<BiomeTagEntry>> {
    let mut tags = FxHashMap::default();
    let Some(vanilla_tags) =
        get_registry_key_tags(JavaMinecraftVersion::V_26_2, RegistryKey::WorldgenBiome)
    else {
        return tags;
    };
    for (raw_id, values) in vanilla_tags.entries() {
        let Ok(id) = Identifier::parse(raw_id) else {
            continue;
        };
        tags.insert(
            id,
            values
                .1
                .iter()
                .filter_map(|id| u8::try_from(*id).ok())
                .map(BiomeTagEntry::Biome)
                .collect(),
        );
    }
    tags
}

impl DataPackManager {
    /// Loads every folder and ZIP data pack found in `<world>/datapacks`.
    ///
    /// Packs are applied in lexical filename order. Later packs replace functions
    /// with the same identifier, while function tags merge unless `replace` is true.
    #[must_use]
    pub fn load(world_path: &Path) -> Self {
        let data_packs_path = world_path.join("datapacks");
        let entries = match fs::read_dir(&data_packs_path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                warn!(
                    "Failed to inspect data pack directory {}: {error}",
                    data_packs_path.display()
                );
                return Self::default();
            }
        };

        let mut paths = entries
            .filter_map(|entry| match entry {
                Ok(entry) => Some(entry.path()),
                Err(error) => {
                    warn!(
                        "Failed to inspect an entry in {}: {error}",
                        data_packs_path.display()
                    );
                    None
                }
            })
            .filter(|path| {
                path.is_dir()
                    || path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
            })
            .collect::<Vec<_>>();
        paths.sort_by(|left, right| left.file_name().cmp(&right.file_name()));

        let mut manager = Self::default();
        let mut loaded_packs = Vec::new();

        for path in paths {
            match load_pack(&path) {
                Ok(pack) => {
                    loaded_packs.push(pack.name.clone());
                    manager.apply_pack(pack);
                }
                Err(error) => warn!("Skipping data pack {}: {error}", path.display()),
            }
        }

        manager.loaded_packs = loaded_packs.into();
        manager.load_functions = manager
            .resolve_tag(&Identifier::vanilla_static("load"))
            .into();
        manager.tick_functions = manager
            .resolve_tag(&Identifier::vanilla_static("tick"))
            .into();
        manager.validate_entity_type_tags();
        manager.validate_biome_tags();
        manager
    }

    #[must_use]
    pub fn function(&self, id: &Identifier) -> Option<&Arc<DataPackFunction>> {
        self.functions.get(id)
    }

    pub fn function_ids(&self) -> impl Iterator<Item = &Identifier> {
        self.functions.keys()
    }

    /// Returns whether an entity type belongs to a vanilla or data-pack-defined
    /// entity type tag.
    #[must_use]
    pub fn entity_type_is_tagged(&self, tag: &Identifier, entity_type: &EntityType) -> bool {
        self.entity_type_tag_contains(tag, entity_type.id, &mut Vec::new())
    }

    fn entity_type_tag_contains(
        &self,
        tag: &Identifier,
        entity_type_id: u16,
        stack: &mut Vec<Identifier>,
    ) -> bool {
        if stack.contains(tag) {
            return false;
        }
        let Some(entries) = self.entity_type_tags.get(tag) else {
            return false;
        };
        stack.push(tag.clone());
        let matches = entries.iter().any(|entry| match entry {
            EntityTypeTagEntry::EntityType(id) => *id == entity_type_id,
            EntityTypeTagEntry::Tag { id, .. } => {
                self.entity_type_tag_contains(id, entity_type_id, stack)
            }
        });
        stack.pop();
        matches
    }

    fn validate_entity_type_tags(&self) {
        for (tag_id, entries) in &self.entity_type_tags {
            for entry in entries {
                if let EntityTypeTagEntry::Tag { id, required: true } = entry
                    && !self.entity_type_tags.contains_key(id)
                {
                    warn!("Required entity type tag #{id} referenced by #{tag_id} does not exist");
                }
            }
        }
    }

    /// Returns whether a biome belongs to a vanilla or data-pack-defined biome tag.
    #[must_use]
    pub fn biome_is_tagged(&self, tag: &Identifier, biome: &Biome) -> bool {
        self.biome_tag_contains(tag, biome.id, &mut Vec::new())
    }

    fn biome_tag_contains(
        &self,
        tag: &Identifier,
        biome_id: u8,
        stack: &mut Vec<Identifier>,
    ) -> bool {
        if stack.contains(tag) {
            return false;
        }
        let Some(entries) = self.biome_tags.get(tag) else {
            return false;
        };
        stack.push(tag.clone());
        let matches = entries.iter().any(|entry| match entry {
            BiomeTagEntry::Biome(id) => *id == biome_id,
            BiomeTagEntry::Tag { id, .. } => self.biome_tag_contains(id, biome_id, stack),
        });
        stack.pop();
        matches
    }

    fn validate_biome_tags(&self) {
        for (tag_id, entries) in &self.biome_tags {
            for entry in entries {
                if let BiomeTagEntry::Tag { id, required: true } = entry
                    && !self.biome_tags.contains_key(id)
                {
                    warn!("Required biome tag #{id} referenced by #{tag_id} does not exist");
                }
            }
        }
    }

    #[must_use]
    pub fn has_tag(&self, id: &Identifier) -> bool {
        self.tags.contains_key(id)
    }

    pub fn tag_ids(&self) -> impl Iterator<Item = &Identifier> {
        self.tags.keys()
    }

    #[must_use]
    pub fn load_functions(&self) -> &[Identifier] {
        &self.load_functions
    }

    #[must_use]
    pub fn tick_functions(&self) -> &[Identifier] {
        &self.tick_functions
    }

    #[must_use]
    pub fn loaded_packs(&self) -> &[String] {
        &self.loaded_packs
    }

    pub async fn run_load_functions(&self, server: &Arc<Server>) {
        if self.loaded_packs.is_empty() {
            return;
        }
        info!(
            "Loaded {} data pack(s), {} function(s), and {} load function(s)",
            self.loaded_packs.len(),
            self.functions.len(),
            self.load_functions.len()
        );
        self.run_tag(server, &self.load_functions).await;
    }

    pub async fn run_tick_functions(&self, server: &Arc<Server>) {
        if !self.tick_functions.is_empty() {
            self.run_tag(server, &self.tick_functions).await;
        }
    }

    pub async fn execute_function(
        &self,
        server: &Arc<Server>,
        id: &Identifier,
        source: &CommandSource,
    ) -> Result<i32, FunctionExecutionError> {
        if FUNCTION_EXECUTION_STATE.try_with(|_| ()).is_ok() {
            self.execute_function_inner(server, id, source).await
        } else {
            let max_commands = server
                .level_info
                .load()
                .game_rules
                .max_command_sequence_length
                .max(0);
            FUNCTION_EXECUTION_STATE
                .scope(
                    FunctionExecutionState {
                        commands: Cell::new(0),
                        depth: Cell::new(0),
                        max_commands,
                    },
                    self.execute_function_inner(server, id, source),
                )
                .await
        }
    }

    pub async fn execute_tag(
        &self,
        server: &Arc<Server>,
        id: &Identifier,
        source: &CommandSource,
    ) -> Result<i32, FunctionExecutionError> {
        if !self.tags.contains_key(id) {
            return Err(FunctionExecutionError::UnknownTag(id.clone()));
        }
        let functions = self.resolve_tag(id);
        let execute = async {
            let mut successful_commands = 0;
            for function in &functions {
                successful_commands += self
                    .execute_function_inner(server, function, source)
                    .await?;
            }
            Ok(successful_commands)
        };

        if FUNCTION_EXECUTION_STATE.try_with(|_| ()).is_ok() {
            execute.await
        } else {
            let max_commands = server
                .level_info
                .load()
                .game_rules
                .max_command_sequence_length
                .max(0);
            FUNCTION_EXECUTION_STATE
                .scope(
                    FunctionExecutionState {
                        commands: Cell::new(0),
                        depth: Cell::new(0),
                        max_commands,
                    },
                    execute,
                )
                .await
        }
    }

    async fn run_tag(&self, server: &Arc<Server>, functions: &[Identifier]) {
        let source = CommandSender::Dummy.into_source(server).await;
        let max_commands = server
            .level_info
            .load()
            .game_rules
            .max_command_sequence_length
            .max(0);
        FUNCTION_EXECUTION_STATE
            .scope(
                FunctionExecutionState {
                    commands: Cell::new(0),
                    depth: Cell::new(0),
                    max_commands,
                },
                async {
                    for id in functions {
                        if let Err(error) = self.execute_function_inner(server, id, &source).await {
                            warn!("Failed to execute data pack function {id}: {error}");
                            break;
                        }
                    }
                },
            )
            .await;
    }

    async fn execute_function_inner(
        &self,
        server: &Arc<Server>,
        id: &Identifier,
        source: &CommandSource,
    ) -> Result<i32, FunctionExecutionError> {
        let function = self
            .functions
            .get(id)
            .cloned()
            .ok_or_else(|| FunctionExecutionError::UnknownFunction(id.clone()))?;

        FUNCTION_EXECUTION_STATE.with(|state| {
            if state.depth.get() >= MAX_FUNCTION_DEPTH {
                return Err(FunctionExecutionError::RecursionLimit);
            }
            state.depth.set(state.depth.get() + 1);
            Ok(())
        })?;

        let mut successful_commands = 0;
        let mut fatal_error = None;
        for function_command in function.commands.iter() {
            let limit_result = FUNCTION_EXECUTION_STATE.with(|state| {
                let command_count = state.commands.get() + 1;
                if command_count > state.max_commands {
                    Err(FunctionExecutionError::CommandLimit(state.max_commands))
                } else {
                    state.commands.set(command_count);
                    Ok(())
                }
            });
            if let Err(error) = limit_result {
                fatal_error = Some(error);
                break;
            }

            let result = {
                let dispatcher = server.command_dispatcher.read().await;
                dispatcher
                    .execute_input_with_fallback(&function_command.command, source)
                    .await
            };
            match result {
                Ok(_) => successful_commands += 1,
                Err(error) => self.report_command_error(&function, function_command, error),
            }
        }

        FUNCTION_EXECUTION_STATE.with(|state| state.depth.set(state.depth.get() - 1));
        fatal_error.map_or(Ok(successful_commands), Err)
    }

    fn report_command_error(
        &self,
        function: &DataPackFunction,
        function_command: &FunctionCommand,
        error: UnifiedCommandError,
    ) {
        let detail = match error {
            UnifiedCommandError::Modern(error) => error.message.get_text(),
            UnifiedCommandError::Legacy(error) => error
                .into_messages(&function_command.command)
                .into_iter()
                .map(pumpkin_util::text::TextComponent::get_text)
                .collect::<Vec<_>>()
                .join("; "),
        };
        let message = format!(
            "{}:{}: command {:?} failed: {detail}",
            function.id, function_command.line, function_command.command
        );
        let should_report = self
            .reported_command_errors
            .lock()
            .is_ok_and(|mut reported| reported.insert(message.clone()));
        if should_report {
            warn!("{message}");
        }
    }

    fn apply_pack(&mut self, mut pack: LoadedPack) {
        pack.files.sort_by(|left, right| left.path.cmp(&right.path));

        for file in pack.files {
            if let Some(id) = function_id_from_path(&file.path) {
                match parse_function(&id, &file.contents) {
                    Ok(function) => {
                        self.functions.insert(id, Arc::new(function));
                    }
                    Err(error) => warn!(
                        "Ignoring invalid function {id} in data pack {}: {error}",
                        pack.name
                    ),
                }
                continue;
            }

            if let Some(tag_id) = function_tag_id_from_path(&file.path) {
                self.apply_function_tag(&pack.name, &tag_id, &file.contents);
            } else if let Some(tag_id) = entity_type_tag_id_from_path(&file.path) {
                self.apply_entity_type_tag(&pack.name, &tag_id, &file.contents);
            } else if let Some(tag_id) = biome_tag_id_from_path(&file.path) {
                self.apply_biome_tag(&pack.name, &tag_id, &file.contents);
            }
        }
    }

    fn apply_function_tag(&mut self, pack_name: &str, tag_id: &Identifier, contents: &[u8]) {
        let tag = match serde_json::from_slice::<TagFile>(contents) {
            Ok(tag) => tag,
            Err(error) => {
                warn!("Ignoring invalid function tag {tag_id} in data pack {pack_name}: {error}");
                return;
            }
        };
        let entries = self.tags.entry(tag_id.clone()).or_default();
        if tag.replace {
            entries.clear();
        }
        for value in tag.values {
            let (raw_id, required) = tag_value_parts(value);
            let (is_tag, raw_id) = raw_id
                .strip_prefix('#')
                .map_or((false, raw_id.as_str()), |id| (true, id));
            match Identifier::parse(raw_id) {
                Ok(id) => entries.push(TagEntry {
                    id,
                    is_tag,
                    required,
                }),
                Err(error) => warn!(
                    "Ignoring invalid identifier {raw_id:?} in function tag {tag_id}: {error}"
                ),
            }
        }
    }

    fn apply_entity_type_tag(&mut self, pack_name: &str, tag_id: &Identifier, contents: &[u8]) {
        let tag = match serde_json::from_slice::<TagFile>(contents) {
            Ok(tag) => tag,
            Err(error) => {
                warn!(
                    "Ignoring invalid entity type tag {tag_id} in data pack {pack_name}: {error}"
                );
                return;
            }
        };
        let entries = self.entity_type_tags.entry(tag_id.clone()).or_default();
        if tag.replace {
            entries.clear();
        }
        for value in tag.values {
            let (raw_id, required) = tag_value_parts(value);
            if let Some(raw_tag_id) = raw_id.strip_prefix('#') {
                match Identifier::parse(raw_tag_id) {
                    Ok(id) => entries.push(EntityTypeTagEntry::Tag { id, required }),
                    Err(error) => warn!(
                        "Ignoring invalid tag identifier {raw_tag_id:?} in entity type tag {tag_id}: {error}"
                    ),
                }
                continue;
            }
            match Identifier::parse(&raw_id) {
                Ok(id) if id.namespace() == "minecraft" => {
                    if let Some(entity_type) = EntityType::from_name(id.path()) {
                        entries.push(EntityTypeTagEntry::EntityType(entity_type.id));
                    } else if required {
                        warn!("Required entity type {id} referenced by #{tag_id} does not exist");
                    }
                }
                Ok(id) if required => {
                    warn!("Required entity type {id} referenced by #{tag_id} is not registered");
                }
                Ok(_) => {}
                Err(error) => warn!(
                    "Ignoring invalid identifier {raw_id:?} in entity type tag {tag_id}: {error}"
                ),
            }
        }
    }

    fn apply_biome_tag(&mut self, pack_name: &str, tag_id: &Identifier, contents: &[u8]) {
        let tag = match serde_json::from_slice::<TagFile>(contents) {
            Ok(tag) => tag,
            Err(error) => {
                warn!("Ignoring invalid biome tag {tag_id} in data pack {pack_name}: {error}");
                return;
            }
        };
        let entries = self.biome_tags.entry(tag_id.clone()).or_default();
        if tag.replace {
            entries.clear();
        }
        for value in tag.values {
            let (raw_id, required) = tag_value_parts(value);
            if let Some(raw_tag_id) = raw_id.strip_prefix('#') {
                match Identifier::parse(raw_tag_id) {
                    Ok(id) => entries.push(BiomeTagEntry::Tag { id, required }),
                    Err(error) => warn!(
                        "Ignoring invalid tag identifier {raw_tag_id:?} in biome tag {tag_id}: {error}"
                    ),
                }
                continue;
            }
            match Identifier::parse(&raw_id) {
                Ok(id) if id.namespace() == "minecraft" => {
                    if let Some(biome) = Biome::from_name(id.path()) {
                        entries.push(BiomeTagEntry::Biome(biome.id));
                    } else if required {
                        warn!("Required biome {id} referenced by #{tag_id} does not exist");
                    }
                }
                Ok(id) if required => {
                    warn!("Required biome {id} referenced by #{tag_id} is not registered");
                }
                Ok(_) => {}
                Err(error) => {
                    warn!("Ignoring invalid identifier {raw_id:?} in biome tag {tag_id}: {error}");
                }
            }
        }
    }

    fn resolve_tag(&self, root: &Identifier) -> Vec<Identifier> {
        let mut resolved = Vec::new();
        let mut stack = Vec::new();
        self.resolve_tag_inner(root, true, &mut stack, &mut resolved);
        resolved
    }

    fn resolve_tag_inner(
        &self,
        tag_id: &Identifier,
        required: bool,
        stack: &mut Vec<Identifier>,
        resolved: &mut Vec<Identifier>,
    ) {
        if stack.contains(tag_id) {
            warn!("Ignoring recursive function tag reference involving #{tag_id}");
            return;
        }

        let Some(entries) = self.tags.get(tag_id) else {
            if required
                && tag_id != &Identifier::vanilla_static("load")
                && tag_id != &Identifier::vanilla_static("tick")
            {
                warn!("Required function tag #{tag_id} does not exist");
            }
            return;
        };

        stack.push(tag_id.clone());
        for entry in entries {
            if entry.is_tag {
                self.resolve_tag_inner(&entry.id, entry.required, stack, resolved);
            } else if self.functions.contains_key(&entry.id) {
                resolved.push(entry.id.clone());
            } else if entry.required {
                warn!(
                    "Required function {} referenced by #{tag_id} does not exist",
                    entry.id
                );
            }
        }
        stack.pop();
    }
}

fn load_pack(path: &Path) -> Result<LoadedPack, String> {
    if path.is_dir() {
        load_directory_pack(path)
    } else {
        load_zip_pack(path)
    }
}

fn load_directory_pack(path: &Path) -> Result<LoadedPack, String> {
    validate_pack_metadata(
        &fs::read(path.join("pack.mcmeta"))
            .map_err(|error| format!("failed to read pack.mcmeta: {error}"))?,
    )?;

    let mut files = Vec::new();
    let data_path = path.join("data");
    if data_path.is_dir() {
        collect_directory_files(path, &data_path, &mut files)?;
    }

    Ok(LoadedPack {
        name: pack_name(path),
        files,
    })
}

fn collect_directory_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<PackFile>,
) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|error| format!("failed to read {}: {error}", current.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if file_type.is_dir() {
            collect_directory_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| format!("failed to resolve {}: {error}", path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            if !is_relevant_data_file(&relative) {
                continue;
            }
            let metadata = entry
                .metadata()
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
            if metadata.len() > MAX_DATA_PACK_FILE_SIZE {
                return Err(format!(
                    "{} exceeds the per-file size limit",
                    path.display()
                ));
            }
            files.push(PackFile {
                path: relative,
                contents: fs::read(&path)
                    .map_err(|error| format!("failed to read {}: {error}", path.display()))?,
            });
        }
    }
    Ok(())
}

fn load_zip_pack(path: &Path) -> Result<LoadedPack, String> {
    let file = File::open(path).map_err(|error| format!("failed to open ZIP: {error}"))?;
    let mut archive = ZipArchive::new(file).map_err(|error| format!("invalid ZIP: {error}"))?;
    let mut files = Vec::new();
    let mut metadata = None;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("failed to inspect ZIP entry: {error}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().trim_start_matches("./").replace('\\', "/");
        if name != "pack.mcmeta" && !is_relevant_data_file(&name) {
            continue;
        }
        if entry.size() > MAX_DATA_PACK_FILE_SIZE {
            return Err(format!("ZIP entry {name} exceeds the per-file size limit"));
        }
        let mut contents = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut contents)
            .map_err(|error| format!("failed to read ZIP entry {name}: {error}"))?;
        if name == "pack.mcmeta" {
            metadata = Some(contents);
        } else {
            files.push(PackFile {
                path: name,
                contents,
            });
        }
    }

    validate_pack_metadata(
        metadata
            .as_deref()
            .ok_or_else(|| "pack.mcmeta is missing from the ZIP root".to_string())?,
    )?;

    Ok(LoadedPack {
        name: pack_name(path),
        files,
    })
}

fn validate_pack_metadata(contents: &[u8]) -> Result<(), String> {
    let metadata: serde_json::Value = serde_json::from_slice(contents)
        .map_err(|error| format!("pack.mcmeta is not valid JSON: {error}"))?;
    let pack = metadata
        .get("pack")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "pack.mcmeta does not contain a pack object".to_string())?;
    let fixed_format = pack.get("pack_format").and_then(serde_json::Value::as_f64);
    let min_format = pack.get("min_format").and_then(serde_json::Value::as_f64);
    let max_format = pack.get("max_format").and_then(serde_json::Value::as_f64);
    let is_compatible = fixed_format.is_some_and(|format| format == CURRENT_DATA_PACK_FORMAT)
        || min_format.zip(max_format).is_some_and(|(min, max)| {
            min <= CURRENT_DATA_PACK_FORMAT && CURRENT_DATA_PACK_FORMAT <= max
        });
    if fixed_format.is_none() && (min_format.is_none() || max_format.is_none()) {
        return Err(
            "pack.mcmeta must contain a numeric pack_format or min_format/max_format range"
                .to_string(),
        );
    }
    if !is_compatible {
        return Err(format!(
            "pack format is not compatible with Pumpkin 26.2 data pack format {CURRENT_DATA_PACK_FORMAT}"
        ));
    }
    Ok(())
}

fn pack_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn is_relevant_data_file(path: &str) -> bool {
    let path = path.replace('\\', "/");
    let extension = Path::new(&path).extension();
    path.starts_with("data/")
        && (extension.is_some_and(|extension| extension.eq_ignore_ascii_case("mcfunction"))
            || (path.contains("/tags/function/")
                && extension.is_some_and(|extension| extension.eq_ignore_ascii_case("json")))
            || (path.contains("/tags/entity_type/")
                && extension.is_some_and(|extension| extension.eq_ignore_ascii_case("json")))
            || (path.contains("/tags/worldgen/biome/")
                && extension.is_some_and(|extension| extension.eq_ignore_ascii_case("json"))))
}

fn function_id_from_path(path: &str) -> Option<Identifier> {
    resource_id_from_path(path, "function", ".mcfunction")
}

fn function_tag_id_from_path(path: &str) -> Option<Identifier> {
    tag_id_from_path(path, "function")
}

fn entity_type_tag_id_from_path(path: &str) -> Option<Identifier> {
    tag_id_from_path(path, "entity_type")
}

fn biome_tag_id_from_path(path: &str) -> Option<Identifier> {
    tag_id_from_path(path, "worldgen/biome")
}

fn tag_id_from_path(path: &str, registry: &str) -> Option<Identifier> {
    let path = path.strip_prefix("data/")?;
    let (namespace, path) = path.split_once('/')?;
    let path = path
        .strip_prefix("tags/")?
        .strip_prefix(registry)?
        .strip_prefix('/')?
        .strip_suffix(".json")?;
    Identifier::new(namespace.to_string(), path.to_string()).ok()
}

fn tag_value_parts(value: TagFileValue) -> (String, bool) {
    match value {
        TagFileValue::Id(id) => (id, true),
        TagFileValue::Detailed { id, required } => (id, required),
    }
}

fn resource_id_from_path(path: &str, directory: &str, suffix: &str) -> Option<Identifier> {
    let path = path.strip_prefix("data/")?;
    let (namespace, path) = path.split_once('/')?;
    let path = path
        .strip_prefix(directory)?
        .strip_prefix('/')?
        .strip_suffix(suffix)?;
    Identifier::new(namespace.to_string(), path.to_string()).ok()
}

fn parse_function(id: &Identifier, contents: &[u8]) -> Result<DataPackFunction, String> {
    let contents = std::str::from_utf8(contents)
        .map_err(|error| format!("function is not valid UTF-8: {error}"))?;
    let mut commands = Vec::new();
    for (index, raw_line) in contents.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('/') {
            return Err(format!(
                "line {line_number} starts with '/', which is not allowed in function files"
            ));
        }
        if line.starts_with('$') {
            return Err(format!(
                "line {line_number} uses a function macro, which is not supported yet"
            ));
        }
        commands.push(FunctionCommand {
            line: line_number,
            command: Arc::from(line),
        });
    }

    Ok(DataPackFunction {
        id: id.clone(),
        commands: commands.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    fn metadata() -> &'static [u8] {
        br#"{"pack":{"description":"test","min_format":88,"max_format":107.1}}"#
    }

    #[test]
    fn loads_directory_functions_and_tags() {
        let temp = tempdir().unwrap();
        let pack = temp.path().join("datapacks/test");
        fs::create_dir_all(pack.join("data/example/function")).unwrap();
        fs::create_dir_all(pack.join("data/minecraft/tags/function")).unwrap();
        fs::write(pack.join("pack.mcmeta"), metadata()).unwrap();
        fs::write(
            pack.join("data/example/function/hello.mcfunction"),
            "# comment\n\nsay hello\n",
        )
        .unwrap();
        fs::write(
            pack.join("data/minecraft/tags/function/load.json"),
            r#"{"values":["example:hello"]}"#,
        )
        .unwrap();

        let manager = DataPackManager::load(temp.path());

        assert_eq!(manager.loaded_packs(), &["test"]);
        assert_eq!(
            manager.load_functions(),
            &[Identifier::parse_static("example:hello")]
        );
        let function = manager
            .function(&Identifier::parse_static("example:hello"))
            .unwrap();
        assert_eq!(function.commands.len(), 1);
        assert_eq!(&*function.commands[0].command, "say hello");
        assert_eq!(function.commands[0].line, 3);
    }

    #[test]
    fn loads_nested_entity_type_tags_on_top_of_vanilla_tags() {
        let temp = tempdir().unwrap();
        let pack = temp.path().join("datapacks/test");
        fs::create_dir_all(pack.join("data/example/tags/entity_type")).unwrap();
        fs::create_dir_all(pack.join("data/minecraft/tags/entity_type")).unwrap();
        fs::write(pack.join("pack.mcmeta"), metadata()).unwrap();
        fs::write(
            pack.join("data/example/tags/entity_type/hostiles.json"),
            r##"{"values":["minecraft:zombie","#minecraft:skeletons"]}"##,
        )
        .unwrap();
        fs::write(
            pack.join("data/minecraft/tags/entity_type/undead.json"),
            r#"{"replace":true,"values":["minecraft:cow"]}"#,
        )
        .unwrap();

        let manager = DataPackManager::load(temp.path());
        let hostiles = Identifier::parse_static("example:hostiles");
        let undead = Identifier::parse_static("minecraft:undead");

        assert!(manager.entity_type_is_tagged(&hostiles, &EntityType::ZOMBIE));
        assert!(manager.entity_type_is_tagged(&hostiles, &EntityType::SKELETON));
        assert!(!manager.entity_type_is_tagged(&hostiles, &EntityType::COW));
        assert!(manager.entity_type_is_tagged(&undead, &EntityType::COW));
        assert!(!manager.entity_type_is_tagged(&undead, &EntityType::ZOMBIE));
    }

    #[test]
    fn default_manager_exposes_vanilla_entity_type_tags() {
        let manager = DataPackManager::default();
        let undead = Identifier::parse_static("minecraft:undead");

        assert!(manager.entity_type_is_tagged(&undead, &EntityType::ZOMBIE));
        assert!(!manager.entity_type_is_tagged(&undead, &EntityType::COW));
    }

    #[test]
    fn loads_data_pack_biome_tags() {
        let temp = tempdir().unwrap();
        let pack = temp.path().join("datapacks/test");
        fs::create_dir_all(pack.join("data/minecraft/tags/worldgen/biome")).unwrap();
        fs::write(pack.join("pack.mcmeta"), metadata()).unwrap();
        fs::write(
            pack.join("data/minecraft/tags/worldgen/biome/is_frozen.json"),
            r#"{"values":["minecraft:frozen_river","minecraft:snowy_plains"]}"#,
        )
        .unwrap();

        let manager = DataPackManager::load(temp.path());
        let frozen = Identifier::parse_static("minecraft:is_frozen");

        assert!(manager.biome_is_tagged(&frozen, &Biome::FROZEN_RIVER));
        assert!(!manager.biome_is_tagged(&frozen, &Biome::PLAINS));
    }

    #[test]
    fn loads_zip_functions() {
        let temp = tempdir().unwrap();
        let data_packs = temp.path().join("datapacks");
        fs::create_dir_all(&data_packs).unwrap();
        let file = File::create(data_packs.join("test.zip")).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("pack.mcmeta", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(metadata()).unwrap();
        zip.start_file(
            "data/example/function/zipped.mcfunction",
            SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"say zipped\n").unwrap();
        zip.finish().unwrap();

        let manager = DataPackManager::load(temp.path());

        assert!(
            manager
                .function(&Identifier::parse_static("example:zipped"))
                .is_some()
        );
    }

    #[test]
    fn later_packs_override_functions_and_replace_tags() {
        let temp = tempdir().unwrap();
        for (name, command, replace) in [("a", "say old", false), ("b", "say new", true)] {
            let pack = temp.path().join("datapacks").join(name);
            fs::create_dir_all(pack.join("data/example/function")).unwrap();
            fs::create_dir_all(pack.join("data/minecraft/tags/function")).unwrap();
            fs::write(pack.join("pack.mcmeta"), metadata()).unwrap();
            fs::write(
                pack.join("data/example/function/shared.mcfunction"),
                command,
            )
            .unwrap();
            fs::write(
                pack.join("data/minecraft/tags/function/tick.json"),
                format!(r#"{{"replace":{replace},"values":["example:shared"]}}"#),
            )
            .unwrap();
        }

        let manager = DataPackManager::load(temp.path());
        let function = manager
            .function(&Identifier::parse_static("example:shared"))
            .unwrap();

        assert_eq!(&*function.commands[0].command, "say new");
        assert_eq!(manager.tick_functions().len(), 1);
    }

    #[test]
    fn rejects_incompatible_pack_format() {
        let temp = tempdir().unwrap();
        let pack = temp.path().join("datapacks/old");
        fs::create_dir_all(&pack).unwrap();
        fs::write(
            pack.join("pack.mcmeta"),
            br#"{"pack":{"description":"old","pack_format":88}}"#,
        )
        .unwrap();

        let manager = DataPackManager::load(temp.path());

        assert!(manager.loaded_packs().is_empty());
    }
}
