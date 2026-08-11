# Gislinge Viking Boat

- **Creator:** Opus Poly
- **Original:** <https://sketchfab.com/3d-models/gislinge-viking-boat-01098ad7973647a9b558f41d2ebc5193>
- **License:** [Creative Commons Attribution 4.0 International](https://creativecommons.org/licenses/by/4.0/)
- **Downloaded:** 2026-08-11
- **Source SHA-256:** `38768A476963CC62CF14726A4593C7ABAF465F75844228014095F7F5B9F70801`
- **Source size:** 28,367,900 bytes

Attribution statement:

> “Gislinge Viking Boat” by Opus Poly, licensed under CC BY 4.0. Source: https://sketchfab.com/3d-models/gislinge-viking-boat-01098ad7973647a9b558f41d2ebc5193

The repository distributes the downloaded GLB unchanged, including its
embedded materials and textures. The source describes a 7.7 m long, 1.5 m
wide vessel with a 0.25 m draught. At runtime Somnium applies a `0.01` root
scale (the source coordinates are centimetre-sized), turns the hull's long
axis onto the engine's `+X` vessel-forward convention, and preserves all 21
mesh-node transforms. The render hierarchy contains 29,035 triangles.

Physics intentionally does not use the detailed render triangles. One stable
7.1 x 1.44 x 0.68 m box proxy and eight distributed surface-query points drive
buoyancy, point drag, propulsion, and the wake. This keeps the high-detail
asset visual-only and avoids noisy triangle-mesh contact in the water solver.
