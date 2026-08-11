use std::env;
use std::path::PathBuf;

fn main() {
    let target = env::var("TARGET").unwrap();

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/jolt_bridge.h");
    println!("cargo:rerun-if-changed=src/jolt_bridge.cpp");

    // Path to the Jolt source code
    let jolt_base = PathBuf::from("../../example_repo/JoltPhysics-master/JoltPhysics-master/Jolt");
    println!("cargo:rerun-if-changed={}", jolt_base.display());

    let mut build = cc::Build::new();

    build
        .cpp(true)
        .std("c++17")
        .include("../../example_repo/JoltPhysics-master/JoltPhysics-master")
        .include("src")
        // Jolt configuration macros
        .define("JPH_OBJECT_LAYER_BITS", "16")
        .define("JPH_CROSS_PLATFORM_DETERMINISTIC", "1"); // Helpful for stability

    // Optional: Use double precision
    // build.define("JPH_DOUBLE_PRECISION", "1");

    if target.contains("msvc") {
        build.flag("/fp:fast").flag("/EHsc").flag("/GR-"); // Disable RTTI
    } else {
        build
            .flag("-ffast-math")
            .flag("-fno-rtti")
            .flag("-fno-exceptions");
    }

    // Add our bridge file
    build.file("src/jolt_bridge.cpp");

    // We only need a subset of Jolt files to get started.
    // Instead of using cmake, we manually add the source files.
    let jolt_sources = [
        "AABBTree/AABBTreeBuilder.cpp",
        "Compute/ComputeSystem.cpp",
        "Core/Color.cpp",
        "Core/Factory.cpp",
        "Core/IssueReporting.cpp",
        "Core/JobSystemSingleThreaded.cpp",
        "Core/JobSystemThreadPool.cpp",
        "Core/JobSystemWithBarrier.cpp",
        "Core/LinearCurve.cpp",
        "Core/Memory.cpp",
        "Core/Profiler.cpp",
        "Core/RTTI.cpp",
        "Core/Semaphore.cpp",
        "Core/StringTools.cpp",
        "Core/TickCounter.cpp",
        "Geometry/ConvexHullBuilder.cpp",
        "Geometry/ConvexHullBuilder2D.cpp",
        "Geometry/Indexify.cpp",
        "Geometry/OrientedBox.cpp",
        "Math/Vec3.cpp",
        "ObjectStream/SerializableObject.cpp",
        "ObjectStream/TypeDeclarations.cpp",
        "ObjectStream/ObjectStream.cpp",
        "ObjectStream/ObjectStreamBinaryIn.cpp",
        "ObjectStream/ObjectStreamBinaryOut.cpp",
        "ObjectStream/ObjectStreamIn.cpp",
        "ObjectStream/ObjectStreamOut.cpp",
        "ObjectStream/ObjectStreamTextIn.cpp",
        "ObjectStream/ObjectStreamTextOut.cpp",
        "Physics/Body/Body.cpp",
        "Physics/Body/BodyCreationSettings.cpp",
        "Physics/Body/BodyInterface.cpp",
        "Physics/Body/BodyManager.cpp",
        "Physics/Body/MassProperties.cpp",
        "Physics/Body/MotionProperties.cpp",
        "Physics/Character/Character.cpp",
        "Physics/Character/CharacterBase.cpp",
        "Physics/Character/CharacterVirtual.cpp",
        "Physics/Collision/BroadPhase/BroadPhase.cpp",
        "Physics/Collision/BroadPhase/BroadPhaseBruteForce.cpp",
        "Physics/Collision/BroadPhase/BroadPhaseQuadTree.cpp",
        "Physics/Collision/BroadPhase/QuadTree.cpp",
        "Physics/Collision/CastConvexVsTriangles.cpp",
        "Physics/Collision/CastSphereVsTriangles.cpp",
        "Physics/Collision/CollideConvexVsTriangles.cpp",
        "Physics/Collision/CollideSphereVsTriangles.cpp",
        "Physics/Collision/CollisionDispatch.cpp",
        "Physics/Collision/CollisionGroup.cpp",
        "Physics/Collision/EstimateCollisionResponse.cpp",
        "Physics/Collision/GroupFilter.cpp",
        "Physics/Collision/GroupFilterTable.cpp",
        "Physics/Collision/ManifoldBetweenTwoFaces.cpp",
        "Physics/Collision/NarrowPhaseQuery.cpp",
        "Physics/Collision/NarrowPhaseStats.cpp",
        "Physics/Collision/PhysicsMaterial.cpp",
        "Physics/Collision/PhysicsMaterialSimple.cpp",
        "Physics/Collision/Shape/BoxShape.cpp",
        "Physics/Collision/Shape/CapsuleShape.cpp",
        "Physics/Collision/Shape/CompoundShape.cpp",
        "Physics/Collision/Shape/ConvexHullShape.cpp",
        "Physics/Collision/Shape/ConvexShape.cpp",
        "Physics/Collision/Shape/CylinderShape.cpp",
        "Physics/Collision/Shape/DecoratedShape.cpp",
        "Physics/Collision/Shape/EmptyShape.cpp",
        "Physics/Collision/Shape/HeightFieldShape.cpp",
        "Physics/Collision/Shape/MeshShape.cpp",
        "Physics/Collision/Shape/MutableCompoundShape.cpp",
        "Physics/Collision/Shape/OffsetCenterOfMassShape.cpp",
        "Physics/Collision/Shape/PlaneShape.cpp",
        "Physics/Collision/Shape/RotatedTranslatedShape.cpp",
        "Physics/Collision/Shape/ScaledShape.cpp",
        "Physics/Collision/Shape/Shape.cpp",
        "Physics/Collision/Shape/SphereShape.cpp",
        "Physics/Collision/Shape/StaticCompoundShape.cpp",
        "Physics/Collision/Shape/TaperedCapsuleShape.cpp",
        "Physics/Collision/Shape/TaperedCylinderShape.cpp",
        "Physics/Collision/Shape/TriangleShape.cpp",
        "Physics/Collision/TransformedShape.cpp",
        "Physics/Constraints/ConeConstraint.cpp",
        "Physics/Constraints/Constraint.cpp",
        "Physics/Constraints/ConstraintManager.cpp",
        "Physics/Constraints/ContactConstraintManager.cpp",
        "Physics/Constraints/DistanceConstraint.cpp",
        "Physics/Constraints/FixedConstraint.cpp",
        "Physics/Constraints/GearConstraint.cpp",
        "Physics/Constraints/HingeConstraint.cpp",
        "Physics/Constraints/MotorSettings.cpp",
        "Physics/Constraints/PathConstraint.cpp",
        "Physics/Constraints/PathConstraintPath.cpp",
        "Physics/Constraints/PathConstraintPathHermite.cpp",
        "Physics/Constraints/PointConstraint.cpp",
        "Physics/Constraints/PulleyConstraint.cpp",
        "Physics/Constraints/RackAndPinionConstraint.cpp",
        "Physics/Constraints/SixDOFConstraint.cpp",
        "Physics/Constraints/SliderConstraint.cpp",
        "Physics/Constraints/SpringSettings.cpp",
        "Physics/Constraints/SwingTwistConstraint.cpp",
        "Physics/Constraints/TwoBodyConstraint.cpp",
        "Physics/DeterminismLog.cpp",
        "Physics/Hair/Hair.cpp",
        "Physics/Hair/HairSettings.cpp",
        "Physics/Hair/HairShaders.cpp",
        "Physics/IslandBuilder.cpp",
        "Physics/LargeIslandSplitter.cpp",
        "Physics/PhysicsScene.cpp",
        "Physics/PhysicsSystem.cpp",
        "Physics/PhysicsUpdateContext.cpp",
        "Physics/Ragdoll/Ragdoll.cpp",
        "Physics/SoftBody/SoftBodyCreationSettings.cpp",
        "Physics/SoftBody/SoftBodyMotionProperties.cpp",
        "Physics/SoftBody/SoftBodyShape.cpp",
        "Physics/SoftBody/SoftBodySharedSettings.cpp",
        "Physics/StateRecorderImpl.cpp",
        "Physics/Vehicle/MotorcycleController.cpp",
        "Physics/Vehicle/TrackedVehicleController.cpp",
        "Physics/Vehicle/VehicleAntiRollBar.cpp",
        "Physics/Vehicle/VehicleCollisionTester.cpp",
        "Physics/Vehicle/VehicleConstraint.cpp",
        "Physics/Vehicle/VehicleController.cpp",
        "Physics/Vehicle/VehicleDifferential.cpp",
        "Physics/Vehicle/VehicleEngine.cpp",
        "Physics/Vehicle/VehicleTrack.cpp",
        "Physics/Vehicle/VehicleTransmission.cpp",
        "Physics/Vehicle/Wheel.cpp",
        "Physics/Vehicle/WheeledVehicleController.cpp",
        "RegisterTypes.cpp",
        "Renderer/DebugRenderer.cpp",
        "Renderer/DebugRendererPlayback.cpp",
        "Renderer/DebugRendererRecorder.cpp",
        "Renderer/DebugRendererSimple.cpp",
        "Skeleton/SkeletalAnimation.cpp",
        "Skeleton/Skeleton.cpp",
        "Skeleton/SkeletonMapper.cpp",
        "Skeleton/SkeletonPose.cpp",
        "TriangleSplitter/TriangleSplitter.cpp",
        "TriangleSplitter/TriangleSplitterBinning.cpp",
        "TriangleSplitter/TriangleSplitterMean.cpp",
    ];

    for src in jolt_sources.iter() {
        build.file(jolt_base.join(src));
    }

    build.compile("joltphysics");
}
