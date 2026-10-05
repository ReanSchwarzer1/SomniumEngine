"""Native Hello coverage of shared work/mirror designer components; no source writes."""
import sys,time,json,uuid
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from pipe_client import BridgeError, EditorClient
root=Path(__file__).resolve().parents[3]
c=EditorClient(root/'target/hello-editor/runtime/authoring-connection.json',timeout=30)
def call(method,params=None):
 r=c.call('authoring.'+method,params or {});assert r.get('ok') is not False,r;return r
def execute(action,**p):return call('execute',dict(action=action,request_id=uuid.uuid4().hex,**p))
def entities():return call('query')['entities']
def fields(e,name):return next(x['fields'] for x in e['components'] if x['id']==name)
def has(e,name):return any(x['id']==name for x in e['components'])
def edit(ops):
 q=call('query');p=call('plan',dict(expected_revision=q['revision'],label='Shared fracture gate',operations=ops));return call('commit',dict(request_id=uuid.uuid4().hex,plan_token=p['plan_token']))
def setfield(e,c,f,v):return dict(op='set',entity=e,component=c,field=f,value=v)
def wait(fn):
 end=time.monotonic()+90
 while time.monotonic()<end:
  try:
   x=fn()
   if x:return x
  except (OSError,ConnectionError,BridgeError):pass
  time.sleep(.1)
 raise AssertionError('Timeout')
def main():
 wait(entities);before={e['id'] for e in entities()};report={}
 try:
  execute('preset',id='hello.work_demo',args={'position':[100,1.4,100]})
  work=[e for e in entities() if has(e,'somnium.WorkTarget') and e['id'] not in before]
  assert len(work)==2
  report['work_fields']=sorted(fields(work[0],'somnium.WorkTarget'))
  result=c.call('authoring.plan',dict(expected_revision=call('query')['revision'],label='Reject runtime mutation',operations=[setfield(work[0]['id'],'somnium.WorkTarget','complete',True)]))
  assert result.get('ok') is False,result
  report['runtime_readonly']=True
  edit([setfield(work[0]['id'],'somnium.WorkTarget','seconds',1.5)])
  execute('preset',id='hello.mirror_demo',args={'position':[105,1.4,100]})
  mirror=next(e for e in entities() if has(e,'somnium.StagedMirror') and e['id'] not in before)
  execute('camera',position=[105,1.5,105],yaw=-90,pitch=0)
  def triangles():return int(fields(next(e for e in entities() if e['id']==mirror['id']),'somnium.StagedMirror')['rendered_triangles'])
  report['mirror_triangles']=wait(triangles)
  execute('select',entity=mirror['id'],frame=False)
  job=execute('capture',include_editor=True)['job']['job_id']
  def capture():
   item=call('jobs',{'action':'get','id':job})['job'];assert item['status'] not in ('failed','cancelled');return item if item['status']=='succeeded' else None
  report['capture']=wait(capture)['result']
  edit([setfield(mirror['id'],'somnium.StagedMirror','enabled',False)])
  time.sleep(.2);assert triangles()==0
  report['mirror_disabled_clears_count']=True
  print('PASS native presets, editable controls, read-only work runtime, reflected animation and counter reset',flush=True)
 finally:
  new=[e['id'] for e in entities() if e['id'] not in before]
  if new:edit([dict(op='delete',entity=e) for e in reversed(new)])
  out=root/'docs/evidence/shared-fractures.json';out.write_text(json.dumps(report,indent=2),encoding='utf-8')
if __name__=='__main__':main()
