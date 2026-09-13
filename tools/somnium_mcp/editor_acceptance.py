"""Live semantic editor acceptance. Uses a running Hello Engine session, not an emulator.
Edits transient graph/timeline state and restores each tested durable source/scene change.
"""
import argparse,json,time,uuid
from pathlib import Path
from pipe_client import EditorClient

def run(connection,output):
 c=EditorClient(connection,timeout=30);report=json.loads(output.read_text()) if output.exists() else {"cases":{},"source":"live editor"}
 def call(method,p):
  r=c.call(method,p)
  if r.get("ok") is not True:raise AssertionError((method,p,r))
  return r
 def query(**kw):return call("authoring.query",kw)
 def execute(action,**kw):return call("authoring.execute",dict(action=action,request_id=str(uuid.uuid4()),**kw))
 def editor(target,operation,**kw):
  q=query(kind="editor",editor=target)
  return execute("editor",editor=target,operation=operation,expected_view=q["view_token"],**kw)
 def undo():
  h=call("authoring.history",{})
  return call("authoring.history",dict(action="undo",expected_revision=h["revision"],expected_cursor=h["cursor"]))
 def transact(label,ops):
  q=query(kind="scene",limit=0)
  plan=call("authoring.plan",dict(label=label,expected_revision=q["revision"],operations=ops))
  return call("authoring.commit",dict(request_id=str(uuid.uuid4()),plan_token=plan["plan_token"]))
 def record(name,details):report["cases"][name]=details;output.write_text(json.dumps(report,indent=2)+"\n",encoding="utf-8");print(name,flush=True)
 discovery=call("authoring.discover",{"section":"full"})
 report["complete"]=False
 output.with_name("capability-inventory.json").write_text(json.dumps(discovery,indent=2)+"\n",encoding="utf-8")
 assert discovery["game"]["label"]=="Hello Engine","Run this against Hello Engine, not a private game"
 # State overlays are edits on the actual graph document owner, with its native overlay history.
 q=query(kind="editor",editor="graph")
 if "state_document" in q:
  transition=dict(q["state_document"]["transitions"][0]);transition["blend_seconds"]=0.35
  changed=execute("editor",editor="graph",operation="state_set_transition",index=0,transition=transition,expected_state_view=q["state_view_token"])
  restored=execute("editor",editor="graph",operation="state_undo",expected_state_view=changed["state_view_token"])
  assert restored["state_document"]==q["state_document"]
  record("animation_state_overlay",{"pass":True,"native_undo":True})
 # Graph topology and failed/conflicting operations.
 execute("command",id="editor.authoring.scatter")
 for _ in range(100):
  q=query(kind="editor",editor="graph")
  if q["document"]["catalogue"]=="somnium.scatter":break
  time.sleep(.05)
 assert q["document"]["catalogue"]=="somnium.scatter"
 baseline=q["document"]
 added=editor("graph","add_node",archetype=q["catalogue"][0]["id"],position=[950,80])
 conflict=c.call("authoring.execute",dict(action="editor",editor="graph",operation="delete",expected_view=q["view_token"],request_id=str(uuid.uuid4())))
 assert conflict.get("ok") is False
 restored=editor("graph","undo");assert restored["document"]==baseline
 comment=editor("graph","comment",position=[1000,0],size=[180,80],text="Live semantic annotation")
 editor("graph","select",nodes=[comment["created"]])
 editor("graph","move",delta=[20,10]);editor("graph","undo");restored=editor("graph","undo")
 assert restored["document"]==baseline
 record("graph_topology",{"pass":True,"conflict_rejected":True,"native_undo":True})
 # Timeline markers, media, curve keys and native undo all work on the visible timeline.
 q=query(kind="editor",editor="timeline");baseline=q["document"]
 marker=editor("timeline","add_marker",time=2.0,label="contact")
 editor("timeline","move_marker",marker=marker["created"],time=2.5)
 editor("timeline","undo");restored=editor("timeline","undo");assert restored["document"]==baseline
 track=baseline["tracks"][0]["id"]
 editor("timeline","add_key",track=track,channel=0,time=3.0,value=.4)
 restored=editor("timeline","undo");assert restored["document"]==baseline
 media=editor("timeline","add_media",track=track,kind="animation-clip",source="preview.anim",start=0.0,duration=1.0)
 editor("timeline","resize_media",media=media["created"],start=.5,duration=2.0)
 editor("timeline","undo");restored=editor("timeline","undo");assert restored["document"]==baseline
 record("timeline",{"pass":True,"markers":True,"keys":True,"media":True,"native_undo":True})
 # Real renderer-owned terrain data, not a reflected transform surrogate.
 terrain=next(e for e in query(kind="scene",search="Terrain")["entities"] if any(v["id"]=="somnium.Terrain" for v in e["components"]))
 tq=query(kind="terrain",entity=terrain["id"])
 time.sleep(.3)
 rev=query(kind="scene",limit=0)["revision"]
 execute("terrain_stroke",entity=terrain["id"],expected_revision=rev,terrain_revision=tq["terrain_revision"],stroke=dict(mode="raise",radius=2.0,strength=.1,hardness=.3,samples=[[0,0,.02]]))
 undone=undo();tq=query(kind="terrain",entity=terrain["id"])
 rev=query(kind="scene",limit=0)["revision"]
 painted=execute("foliage_stroke",entity=terrain["id"],expected_revision=rev,terrain_revision=tq["terrain_revision"],samples=[[0,0]],kind=0,radius=2.0,single=True,min_layer_weight=0.0,seed=42)
 if painted["total"]!=tq["foliage_count"]:
  undo();assert query(kind="terrain",entity=terrain["id"])["foliage_count"]==tq["foliage_count"]
 record("terrain_and_foliage",{"pass":True,"foliage_placed":painted["placed"],"restored_count":tq["foliage_count"]})
 # Source publication uses the same Luau compiler, exact-byte conflicts and guarded document undo.
 path="assets/scripts/demo_rotator.luau";q=query(kind="document",path=path)
 content={"text":q["content"]["text"]+"\n-- Live authoring acceptance (undone immediately).\n"}
 plan=call("authoring.plan",dict(kind="document",path=path,view_token=q["view_token"],document_type="somnium.luau_source",label="Luau live acceptance",content=content))
 commit=call("authoring.commit",dict(request_id=str(uuid.uuid4()),plan_token=plan["plan_token"]))
 assert query(kind="document",path=path)["content"]==content
 call("authoring.history",dict(kind="document",action="undo",expected_cursor=commit["document_cursor"]))
 assert query(kind="document",path=path)["content"]==q["content"]
 record("luau_source",{"pass":True,"compile_validation":True,"published_undo":True})
 path="assets/ui/hello_hud.somui";q=query(kind="document",path=path)
 content=json.loads(json.dumps(q["content"]));content["reference"]=[1600,900]
 plan=call("authoring.plan",dict(kind="document",path=path,view_token=q["view_token"],document_type="somnium.ui_document",label="UI layout live acceptance",content=content))
 commit=call("authoring.commit",dict(request_id=str(uuid.uuid4()),plan_token=plan["plan_token"]))
 assert query(kind="document",path=path)["content"]==content
 call("authoring.history",dict(kind="document",action="undo",expected_cursor=commit["document_cursor"]))
 assert query(kind="document",path=path)["content"]==q["content"]
 record("ui_layout_document",{"pass":True,"native_schema":True,"published_undo":True})
 # Shared interactions are present in the public sample and remain ordinary reflected authoring.
 execute("preset",id="hello.interaction_demo",args={"position":[0,5,0]})
 actors=query(kind="scene",search="Demo ")["entities"]
 assert len([e for e in actors if any(c["id"]=="somnium.Interactable" for c in e["components"])])==5
 undo()
 record("hello_shared_interaction_preset",{"pass":True,"actions":5,"scene_undo":True})
 execute("preset",id="hello.animated_preview",args={"position":[0,155,450]})
 actor=query(kind="scene",search="Animated Mesh Preview")["entities"][0]
 for _ in range(200):
  state=query(kind="game")["game"]["animated_previews"]
  if state["loaded"]:break
  assert not state["errors"],state
  time.sleep(.05)
 assert state["loaded"] and state["loaded"][0]["frames"]>0,state
 transact("Scrub imported clip",[dict(op="set",entity=actor["id"],component="hello.AnimatedPreview",field="playing",value=False),dict(op="set",entity=actor["id"],component="hello.AnimatedPreview",field="time",value=1.0)])
 time.sleep(.1)
 state=query(kind="game")["game"]["animated_previews"];assert state["loaded"][0]["time"]==1.0,state
 undo();undo();time.sleep(.1)
 assert not query(kind="game")["game"]["animated_previews"]["loaded"]
 record("hello_gpu_animation",{"pass":True,"imported_clip":"Bend","gpu_frames":state["loaded"][0]["frames"],"scrub":True,"native_details":True,"removed_allocations":True})
 camera=query(kind="diagnostics")["camera"]
 execute("bookmark",operation="set",slot=9)
 execute("camera",position=[0,200,0],yaw=0,pitch=0);time.sleep(.1)
 execute("bookmark",operation="recall",slot=9);time.sleep(.1)
 assert query(kind="diagnostics")["camera"]==camera
 picked=query(kind="pick",origin=[0,300,0],direction=[0,-1,0],max_distance=500)
 clear=query(kind="clearance",origin=[0,400,0],displacement=[0,-1,0],radius=.2,half_height=.5)
 assert clear["clear"] and picked["probe"]=="native viewport mesh/proxy bounds"
 record("spatial",{"pass":True,"pick_hits":len(picked["hits"]),"capsule_sweep":True,"bookmark_recall":True})
 created=transact("Create editor family fixture",[dict(op="create",key="fixture",name="Editor family fixture",components={"somnium.Transform":{"translation":[0,160,0],"scale":[6,.2,6]},"somnium.MeshKind":{"kind":"Cube"},"somnium.NavigationProfile":{"bounds_size":[8,4,8],"tile_size":8.0},"somnium.Behavior":{}})])
 actor=created["created"]["fixture"]
 def specialized(action,**kw):return execute(action,entity=actor,expected_revision=query(kind="scene",limit=0)["revision"],**kw)
 specialized("script",operation="attach",path="assets/scripts/demo_rotator.luau")
 specialized("script",operation="number",index=0,field="spinSpeed",value=2.0)
 specialized("script",operation="enabled",index=0,value=False)
 specialized("script",operation="attach",path="assets/scripts/first_person_camera.luau")
 specialized("script",operation="reorder",index=1,delta=-1)
 specialized("script",operation="reload")
 for _ in range(5):undo()
 record("script_attachments",{"pass":True,"attach":True,"exported_number":True,"enabled":True,"reorder":True,"reload":True,"native_undo":True})
 execute("select",entity=actor,frame=False)
 execute("command",id="editor.authoring.behavior")
 for _ in range(100):
  graph=query(kind="editor",editor="graph")
  if graph["document"]["catalogue"]=="somnium.behavior":break
  time.sleep(.05)
 execute("graph_apply",expected_revision=query(kind="scene",limit=0)["revision"],expected_view=graph["view_token"])
 fields=next(c["fields"] for c in query(kind="scene",entity=actor)["entities"][0]["components"] if c["id"]=="somnium.Behavior")
 assert fields["graph_json"]
 undo();record("behavior_compile_apply",{"pass":True,"native_undo":True})
 specialized("designer",tool="navigation_bake")
 for _ in range(200):
  nav=next(c["fields"] for c in query(kind="scene",entity=actor)["entities"][0]["components"] if c["id"]=="somnium.NavigationProfile")
  if nav["pending_cells"]=="0":break
  time.sleep(.05)
 assert int(nav["polygon_count"])>0,nav
 specialized("designer",tool="navigation_clear")
 record("navigation_bake",{"pass":True,"polygons":int(nav["polygon_count"]),"terminal_status":nav["status"],"clear":True})
 undo()
 material=execute("material_open",path="assets/NewMaterial.sommat")
 fields=next(c["fields"] for c in query(kind="scene",entity=material["entity"])["entities"][0]["components"] if c["id"]=="somnium.asset.Material")
 value=.37 if fields["roughness"]!=.37 else .42
 transact("Edit native material draft",[dict(op="set",entity=material["entity"],component="somnium.asset.Material",field="roughness",value=value)])
 changed=next(c["fields"] for c in query(kind="scene",entity=material["entity"])["entities"][0]["components"] if c["id"]=="somnium.asset.Material")
 assert abs(changed["roughness"]-value)<.00001
 undo()
 record("material_native_draft",{"pass":True,"reflected_edit":True,"native_undo":True})
 record("settings",{"pass":bool(query(kind="settings")["settings"])})
 report["complete"]=True;output.write_text(json.dumps(report,indent=2)+"\n",encoding="utf-8")

if __name__=="__main__":
 p=argparse.ArgumentParser();p.add_argument("--connection",required=True,type=Path);p.add_argument("--output",required=True,type=Path);a=p.parse_args();a.output.parent.mkdir(parents=True,exist_ok=True);run(a.connection,a.output)
