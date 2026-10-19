//! Host side of `somnium.CellStreaming` (`cell_streaming.rs`).
//!
//! A cell's reader thread parses the cell, then decodes every glTF source it draws that is not uploaded yet and
//! prepares it for upload (texture keys and cached BC7 chains, mesh clusters: `SomniumRenderer::prepare_scene_for`),
//! one source at a time. Each frame the main thread spends a few milliseconds on what has arrived: a source's GPU
//! writes, or a finished cell's entities (their meshes all cached by then, so installing is cheap). Cells leave by
//! despawning; their meshes stay cached for the next cell that uses them.
use super::*;
use crate::cell_streaming::{self as cells, CellStreamingComponent, Piece, Streamed};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Main-thread time a frame gives streaming once it has done one piece.
const FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(6);
/// The longest a first fill may hold the frame before the rest streams in as usual.
const FIRST_FILL_LIMIT: std::time::Duration = std::time::Duration::from_secs(90);

impl<G: GameApp> Engine<G> {
    /// Stream the cells of the scene's `somnium.CellStreaming` around this frame's view.
    pub(super) fn update_cell_streaming(&mut self) {
        let Some(owner) = self.world.entities_with::<CellStreamingComponent>().next() else {
            if self.cell_streamer.key.is_some() {
                self.reset_cell_streaming();
            }
            return;
        };
        let Some(settings) = self.world.get::<CellStreamingComponent>(owner).cloned() else {
            return;
        };
        let manifest = match self.imported_source_path(&settings.manifest) {
            Ok(path) => path,
            Err(error) => {
                self.set_cell_status(owner, format!("Manifest: {error}"));
                return;
            }
        };
        let key = Some((owner, manifest.clone()));
        if self.cell_streamer.key != key {
            self.reset_cell_streaming();
            self.cell_streamer.key = key;
            match cells::read_manifest(&manifest) {
                Ok(list) => self.cell_streamer.cells = list,
                Err(error) => {
                    self.set_cell_status(owner, error);
                    return;
                }
            }
        }

        // Something else removed a cell's entities (Stop putting the world back, a scene open): forget that cell.
        let gone: Vec<usize> = self
            .cell_streamer
            .loaded
            .iter()
            .filter(|(_, entities)| entities.first().is_some_and(|e| !self.world.is_alive(*e)))
            .map(|(index, _)| *index)
            .collect();
        for index in gone {
            self.unload_cell(index);
        }

        // The camera, unless the view is pinned (a protagonist switch's camera is up, crossing land nobody sees).
        let pinned = self.world.take_cache::<cells::Holds>();
        let focus = pinned.pinned().or_else(|| self.renderer.as_ref().map(|renderer| renderer.camera_pos));
        self.world.put_cache(pinned);
        let first_fill = !self.cell_streamer.filled;
        self.schedule_cells(&settings, focus);
        if first_fill && !self.cell_streamer.reading.is_empty() {
            self.cell_streamer.filled = true;
            // Nothing is in yet (a scene just opened, Play just began): the view's own cells come in whole before the
            // world is shown, as behind a loading screen, rather than popping in a few a frame.
            let started = std::time::Instant::now();
            // Round the readers without a frame budget, so all of them keep decoding while this thread uploads.
            while !self.cell_streamer.reading.is_empty() && started.elapsed() < FIRST_FILL_LIMIT {
                if self.pump_cell_reads(usize::MAX) {
                    // Uploads stage CPU copies until a submit; a fill is many frames' uploads in one, so it flushes as
                    // it goes (a 7-window Tyffir peaked at 15.8 of 16 GB without this).
                    if let Some((renderer, ctx)) = self.renderer.as_mut().zip(self.render_ctx.as_ref()) {
                        ctx.queue.submit(std::iter::empty());
                        renderer.wait_gpu(ctx);
                    }
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                self.schedule_cells(&settings, focus);
            }
            info!(
                cells = self.cell_streamer.loaded.len(),
                entities = self.cell_streamer.streamed_entities(),
                seconds = format!("{:.1}", started.elapsed().as_secs_f64()),
                "World cells: first fill"
            );
        } else {
            self.pump_cell_reads(settings.cells_per_frame.max(1) as usize);
        }

        self.evict_cold_sources();

        let streamer = &self.cell_streamer;
        let (loaded, pending, entities) = (
            u32::try_from(streamer.loaded.len()).unwrap_or(u32::MAX),
            u32::try_from(streamer.reading.len()).unwrap_or(u32::MAX),
            u32::try_from(streamer.streamed_entities()).unwrap_or(u32::MAX),
        );
        let status = match streamer.failed.values().next() {
            Some(error) => format!("{} cells failed; first: {error}", streamer.failed.len()),
            None if !settings.enabled => "Off: cells unloaded".into(),
            None => format!("{loaded} of {} cells live", streamer.cells.len()),
        };
        if let Some(component) = self.world.get::<CellStreamingComponent>(owner)
            && (component.loaded_cells, component.pending_cells, component.streamed_entities) != (loaded, pending, entities)
            && let Some(component) = self.world.get_mut::<CellStreamingComponent>(owner)
        {
            component.loaded_cells = loaded;
            component.pending_cells = pending;
            component.streamed_entities = entities;
        }
        self.set_cell_status(owner, status);
    }

    /// Unload what the view has left and start readers (up to `READERS`) for what it has reached, nearest first. The
    /// points held with `cell_streaming::hold` count as views too, and are marked ready once their cells are in.
    fn schedule_cells(&mut self, settings: &CellStreamingComponent, focus: Option<glam::Vec3>) {
        let mut holds: cells::Holds = self.world.take_cache();
        let streamer = &self.cell_streamer;
        let (load, unload) = match focus {
            Some(focus) if settings.enabled => cells::plan_many(
                &streamer.cells,
                [focus.x, focus.z],
                settings.load_radius,
                settings.unload_margin,
                &holds.discs(),
                |index| streamer.loaded.contains_key(&index) || streamer.reading.contains_key(&index),
            ),
            Some(_) => (Vec::new(), streamer.loaded.keys().chain(streamer.reading.keys()).copied().collect()),
            None => (Vec::new(), Vec::new()),
        };
        for index in unload {
            self.unload_cell(index);
        }
        for index in load {
            if self.cell_streamer.reading.len() >= cells::READERS {
                break;
            }
            if !self.cell_streamer.failed.contains_key(&index) {
                self.start_cell_read(index);
            }
        }
        let streamer = &self.cell_streamer;
        holds.mark_ready(&streamer.cells, |index| {
            streamer.loaded.contains_key(&index) || streamer.failed.contains_key(&index)
        });
        self.world.put_cache(holds);
    }

    /// Start a cell's reader: parse it, then decode and prepare each source it needs that is not uploaded yet.
    fn start_cell_read(&mut self, index: usize) {
        let path = self.cell_streamer.cells[index].file.clone();
        let root = self.config.project_root.clone();
        let uploaded = self.imported_uploads.source_paths();
        let compressing = self
            .render_ctx
            .as_ref()
            .is_some_and(somnium_renderer::SomniumRenderer::imported_compression);
        let drawing = self.renderer.is_some();
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let document = match crate::scene_file::read(&path) {
                Ok((_, document)) => document,
                Err(error) => {
                    let _ = send.send(Piece::Cell(Err(format!("{}: {error}", path.display()))));
                    return;
                }
            };
            let project = root.as_deref().map(crate::authoring::project::ProjectPaths::open);
            for (source, parts) in cells::sources_of(&document).into_iter().filter(|_| drawing) {
                let resolved = match &project {
                    Some(Ok(project)) => project.resolve(Path::new(&source)),
                    Some(Err(error)) => Err(error.clone()),
                    None => Ok(PathBuf::from(&source)),
                };
                let Ok(resolved) = resolved else {
                    continue; // the install reports it, as a scene open would
                };
                if resolved.canonicalize().is_ok_and(|canonical| uploaded.contains(&canonical)) {
                    continue;
                }
                let Ok(scene) = somnium_asset::load_gltf_for(&resolved, Some(&parts)) else {
                    continue; // the install retries it on the main thread and logs why
                };
                let prepared = somnium_renderer::SomniumRenderer::prepare_scene_for(&scene, Some(&parts), compressing);
                if send.send(Piece::Source { path: resolved, parts, scene, prepared }).is_err() {
                    return; // the cell was dropped
                }
            }
            let _ = send.send(Piece::Cell(Ok(document)));
        });
        self.cell_streamer.reading.insert(index, receive);
    }

    /// Spend this frame's streaming budget on what the readers have sent: at least one piece, then until the
    /// budget runs out or `limit` cells are in. True when anything arrived.
    fn pump_cell_reads(&mut self, limit: usize) -> bool {
        let started = std::time::Instant::now();
        let mut indices: Vec<usize> = self.cell_streamer.reading.keys().copied().collect();
        indices.sort_unstable();
        let (mut installed, mut pieces) = (0, 0);
        for index in indices {
            let Some(receive) = self.cell_streamer.reading.remove(&index) else {
                continue;
            };
            let mut finished = false;
            while installed < limit && (pieces == 0 || started.elapsed() < FRAME_BUDGET) {
                match receive.try_recv() {
                    Ok(Piece::Source { path, parts, scene, prepared }) => {
                        pieces += 1;
                        self.upload_streamed_source(&path, &parts, &scene, prepared);
                    }
                    Ok(Piece::Cell(parsed)) => {
                        pieces += 1;
                        installed += 1;
                        finished = true;
                        if let Err(error) = parsed.and_then(|document| self.install_cell(index, &document)) {
                            warn!(%error, cell = index, "streamed cell skipped");
                            self.cell_streamer.failed.insert(index, error);
                        }
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        finished = true;
                        self.cell_streamer.failed.insert(index, "cell reader stopped".into());
                        break;
                    }
                }
            }
            if !finished {
                self.cell_streamer.reading.insert(index, receive);
            }
        }
        pieces > 0
    }

    /// A decoded, prepared source: its GPU writes, into the same cache a scene open uses. Another cell's reader may
    /// have brought it first; then this one is dropped.
    fn upload_streamed_source(
        &mut self,
        path: &Path,
        parts: &HashSet<usize>,
        scene: &somnium_asset::LoadedScene,
        prepared: somnium_renderer::PreparedScene,
    ) {
        let stamp = import_cache::SourceStamp::read(path);
        if stamp.as_ref().is_some_and(|stamp| self.imported_uploads.get(stamp).is_some()) {
            return;
        }
        let Some((renderer, ctx)) = self.renderer.as_mut().zip(self.render_ctx.as_ref()) else {
            return;
        };
        let nodes = renderer.upload_prepared_scene_for(ctx, scene, Some(parts), prepared);
        if let Some(stamp) = stamp {
            self.imported_uploads.insert(stamp, &nodes, Some(parts.clone()));
        }
    }

    /// Put a parsed cell's entities into the world and restore their GPU state (from the cache, as a rule).
    fn install_cell(&mut self, index: usize, document: &serde_json::Value) -> Result<(), String> {
        let report = crate::scene_schema::scene_from_json(&mut self.world, &self.type_registry, document)
            .map_err(|error| error.to_string())?;
        // Which uploads this cell keeps alive (for eviction once no loaded cell draws them).
        let sources: Vec<PathBuf> = cells::sources_of(document)
            .into_keys()
            .filter_map(|source| self.imported_source_path(&source).ok()?.canonicalize().ok())
            .collect();
        for source in &sources {
            *self.cell_streamer.source_users.entry(source.clone()).or_default() += 1;
            self.cell_streamer.cold.remove(source);
        }
        self.cell_streamer.cell_sources.insert(index, sources);
        let cell = u32::try_from(index).unwrap_or(u32::MAX);
        for &entity in &report.entities {
            let _ = self.world.insert_component(entity, Streamed { cell });
        }
        self.restore_entity_gpu(&report.entities);
        // ponytail: both passes walk the whole world once per installed cell; per-entity variants if they show up in
        // a profile.
        self.sync_authored_material_components();
        crate::propagate_transforms(&mut self.world);
        self.cell_streamer.loaded.insert(index, report.entities);
        Ok(())
    }

    /// Despawn a cell's entities (or drop its reader). Its meshes stay cached for the next cell that uses them.
    fn unload_cell(&mut self, index: usize) {
        self.cell_streamer.reading.remove(&index);
        let now = std::time::Instant::now();
        for source in self.cell_streamer.cell_sources.remove(&index).unwrap_or_default() {
            let users = self.cell_streamer.source_users.entry(source.clone()).or_default();
            *users = users.saturating_sub(1);
            if *users == 0 {
                self.cell_streamer.cold.insert(source, now);
            }
        }
        if let Some(entities) = self.cell_streamer.loaded.remove(&index) {
            for entity in entities.into_iter().rev() {
                if self.world.is_alive(entity) {
                    self.world.despawn(entity);
                }
            }
        }
    }

    /// Release streamed meshes no loaded cell has drawn for `COLD_SECONDS`, in one batch at most every few seconds
    /// (each batch waits for the GPU). Sources a scene's own (unstreamed) entities draw are never released here.
    fn evict_cold_sources(&mut self) {
        let now = std::time::Instant::now();
        if self.cell_streamer.cold.is_empty()
            || self.cell_streamer.last_eviction.is_some_and(|t| now.duration_since(t).as_secs() < 5)
        {
            return;
        }
        let due: Vec<PathBuf> = self
            .cell_streamer
            .cold
            .iter()
            .filter(|(_, since)| now.duration_since(**since).as_secs() >= cells::COLD_SECONDS)
            .map(|(source, _)| source.clone())
            .collect();
        if due.is_empty() {
            return;
        }
        self.cell_streamer.last_eviction = Some(now);
        // The scene's own entities' sources stay (one scan per batch, which is rare).
        let pinned: HashSet<PathBuf> = self
            .world
            .entities_with::<crate::ImportedMesh>()
            .filter(|e| self.world.get::<Streamed>(*e).is_none())
            .filter_map(|e| self.world.get::<crate::ImportedMesh>(e).map(|m| m.source.clone()))
            .collect::<HashSet<String>>()
            .into_iter()
            .filter_map(|source| self.imported_source_path(&source).ok()?.canonicalize().ok())
            .collect();
        let (mut meshes, mut materials) = (Vec::new(), Vec::new());
        let mut released = 0;
        for source in due {
            self.cell_streamer.cold.remove(&source);
            self.cell_streamer.source_users.remove(&source);
            if pinned.contains(&source) {
                continue;
            }
            if let Some(nodes) = self.imported_uploads.remove(&source) {
                released += 1;
                meshes.extend(nodes.iter().filter(|n| n.index_count > 0).map(|n| n.vertex_offset));
                materials.extend(nodes.iter().map(|n| n.material_id));
            }
        }
        if released == 0 {
            return;
        }
        let Some((renderer, ctx)) = self.renderer.as_mut().zip(self.render_ctx.as_ref()) else {
            return;
        };
        renderer.wait_gpu(ctx);
        meshes.sort_unstable();
        meshes.dedup();
        materials.sort_unstable();
        materials.dedup();
        let (meshes, textures) = renderer.release_uploads(ctx, meshes, materials);
        info!(sources = released, meshes, textures, "World cells: released cold meshes");
    }

    fn reset_cell_streaming(&mut self) {
        let indices: Vec<usize> = self.cell_streamer.loaded.keys().copied().collect();
        for index in indices {
            self.unload_cell(index);
        }
        self.cell_streamer = crate::cell_streaming::CellStreamer::default();
    }

    fn set_cell_status(&mut self, owner: Entity, status: String) {
        if self.world.get::<CellStreamingComponent>(owner).is_some_and(|c| c.status != status)
            && let Some(component) = self.world.get_mut::<CellStreamingComponent>(owner)
        {
            component.status = status;
        }
    }
}
