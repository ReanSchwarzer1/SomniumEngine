// Project palette adapter: native terrain instances share one upload per source.
impl<G: GameApp> Engine<G> {
    fn foliage_kind_exists(&self, kind: u8) -> bool {
        (kind as usize) < FOLIAGE_PALETTE.len() || self.project_foliage.contains_key(&kind)
    }

    fn foliage_kind_name(&self, kind: u8) -> &str {
        self.project_foliage
            .get(&kind)
            .map(|e| e.name.as_str())
            .or_else(|| FOLIAGE_PALETTE.get(kind as usize).map(|e| e.name))
            .unwrap_or("Unknown foliage")
    }

    fn foliage_palette_labels(&self) -> Vec<(u8, String)> {
        FOLIAGE_PALETTE
            .iter()
            .enumerate()
            .map(|(i, e)| (i as u8, e.name.to_owned()))
            .chain(
                self.project_foliage
                    .iter()
                    .map(|(&id, e)| (id, e.name.clone())),
            )
            .collect()
    }

    fn ensure_project_palette_mesh(&mut self, kind: u8) {
        let index = kind as usize;
        if self.foliage_meshes[index].is_some() || self.foliage_failed[index] {
            return;
        }
        let Some(entry) = self.project_foliage.get(&kind).cloned() else {
            warn!(kind, "Painted foliage kind has no project palette entry");
            self.foliage_failed[index] = true;
            return;
        };
        if !self.project_foliage_sources.contains_key(&entry.source) {
            let Some((renderer, ctx)) = self.renderer.as_mut().zip(self.render_ctx.as_ref()) else {
                return;
            };
            let scene = match somnium_asset::load_gltf(&entry.source) {
                Ok(scene) => scene,
                Err(error) => {
                    warn!(%error, source=%entry.source.display(),"Project foliage import failed");
                    self.foliage_failed[index] = true;
                    return;
                }
            };
            // Project materials are authored content. Preserve their alpha,
            // transmission and roughness instead of applying legacy palette guesses.
            let uploaded = renderer.upload_scene(ctx, &scene);
            let parts = uploaded
                .iter()
                .map(|node| FoliagePart {
                    vertex_offset: node.vertex_offset,
                    index_offset: node.index_offset,
                    index_count: node.index_count,
                    material_id: node.material_id,
                    local: node.transform,
                    is_leaf: scene
                        .materials
                        .get(node.material_index)
                        .is_some_and(|material| {
                            material.foliage
                                || material.alpha_mode != somnium_asset::AlphaMode::Opaque
                        }),
                })
                .collect();
            self.project_foliage_sources
                .insert(entry.source.clone(), parts);
        }
        let source = &self.project_foliage_sources[&entry.source];
        let adjustment = entry
            .local_transform
            .map_or(glam::Mat4::IDENTITY, |m| glam::Mat4::from_cols_array(&m));
        let mut parts = Vec::with_capacity(entry.primitives.len());
        for ordinal in &entry.primitives {
            let Some(mut part) = source.get(*ordinal as usize).copied() else {
                warn!(
                    kind,
                    ordinal, "Project foliage primitive ordinal is outside its source"
                );
                self.foliage_failed[index] = true;
                return;
            };
            part.local = adjustment * part.local;
            parts.push(part);
        }
        info!(kind,name=%entry.name,parts=parts.len(),"Project foliage palette entry ready");
        self.foliage_meshes[index] = Some(parts);
    }
}
