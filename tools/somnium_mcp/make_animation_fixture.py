"""Generate the original CC0 two-joint GPU preview fixture. No third-party art."""
import base64, json, math, struct
from pathlib import Path

def generate(path):
    blob=bytearray(); views=[]; accessors=[]
    def add(values,fmt,kind,components,minimum=None,maximum=None):
        while len(blob)%4:blob.append(0)
        offset=len(blob)
        flat=[x for v in values for x in v] if isinstance(values[0],(list,tuple)) else values
        blob.extend(struct.pack('<'+fmt*len(flat),*flat))
        views.append(dict(buffer=0,byteOffset=offset,byteLength=len(blob)-offset))
        item=dict(bufferView=len(views)-1,componentType=components,count=len(values),type=kind)
        if minimum is not None:item['min']=minimum
        if maximum is not None:item['max']=maximum
        accessors.append(item);return len(accessors)-1
    positions=[];normals=[];uv=[];weights=[];joints=[];indices=[]
    rings=20;sides=12
    for y in range(rings+1):
        height=y/rings*2
        for n in range(sides):
            angle=n/sides*math.tau;x=math.cos(angle);z=math.sin(angle)
            positions.append((x*.18,height,z*.18));normals.append((x,0,z));uv.append((n/sides,y/rings))
            w=max(0,min(1,(height-.6)/.8));weights.append((1-w,w,0,0));joints.append((0,1,0,0))
    for y in range(rings):
        for n in range(sides):
            a=y*sides+n;b=y*sides+(n+1)%sides;c=a+sides;d=b+sides
            indices.extend((a,c,b,b,c,d))
    identity=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1];inverse=identity.copy();inverse[13]=-1
    attrs={'POSITION':add(positions,'f','VEC3',5126,[-.18,0,-.18],[.18,2,.18]),'NORMAL':add(normals,'f','VEC3',5126),'TEXCOORD_0':add(uv,'f','VEC2',5126),'JOINTS_0':add(joints,'H','VEC4',5123),'WEIGHTS_0':add(weights,'f','VEC4',5126)}
    ix=add(indices,'I','SCALAR',5125);ibm=add([identity,inverse],'f','MAT4',5126)
    times=add([0,1,2,3],'f','SCALAR',5126,[0],[3])
    quats=[(0,0,math.sin(a/2),math.cos(a/2)) for a in [0,1,-1,0]]
    rotations=add(quats,'f','VEC4',5126)
    doc=dict(asset={'version':'2.0','generator':'Somnium original CC0 animation fixture'},scene=0,scenes=[{'nodes':[0,2]}],nodes=[{'name':'root','children':[1]},{'name':'tip','translation':[0,1,0]},{'name':'Preview','mesh':0,'skin':0}],skins=[{'joints':[0,1],'skeleton':0,'inverseBindMatrices':ibm}],meshes=[{'primitives':[{'attributes':attrs,'indices':ix,'material':0}]}],materials=[{'name':'Preview Amber','pbrMetallicRoughness':{'baseColorFactor':[.9,.25,.035,1],'metallicFactor':.15,'roughnessFactor':.5}}],animations=[{'name':'Bend','samplers':[{'input':times,'output':rotations,'interpolation':'LINEAR'}],'channels':[{'sampler':0,'target':{'node':1,'path':'rotation'}}]}],bufferViews=views,accessors=accessors,buffers=[{'byteLength':len(blob),'uri':'data:application/octet-stream;base64,'+base64.b64encode(blob).decode()}])
    path.parent.mkdir(parents=True,exist_ok=True);path.write_text(json.dumps(doc,indent=2)+'\n',encoding='utf-8',newline='\n')
if __name__=='__main__':generate(Path(__file__).resolve().parents[2]/'assets/models/animation_demo.gltf')
