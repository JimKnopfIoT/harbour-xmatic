import re,sys,json
def parse(path):
    txt=open(path,encoding='utf-8',errors='replace').read()
    comps=[]
    # crude tokenizer: split top-level Component { ... }
    i=0; stack=[]; cur=None
    lines=txt.split('\n')
    depth=0; comp=None; member=None; mdepth=0
    for ln in lines:
        s=ln.strip()
        if re.match(r'^(Component|ModuleApi)\s*\{',s) and depth==1:
            comp={'name':None,'proto':None,'exports':[],'revs':[],'members':[]}; comps.append(comp)
        m=re.match(r'^(Property|Signal|Method)\s*\{(.*)\}?$',s)
        if comp is not None and depth==2 and m:
            member={'kind':m.group(1),'name':None,'rev':0}; comp['members'].append(member)
            rest=m.group(2)
            nm=re.search(r'name:\s*"([^"]+)"',rest); 
            if nm: member['name']=nm.group(1)
            rv=re.search(r'revision:\s*(\d+)',rest)
            if rv: member['rev']=int(rv.group(1))
        elif comp is not None and depth==3 and member is not None:
            nm=re.match(r'name:\s*"([^"]+)"',s)
            if nm and member['name'] is None: member['name']=nm.group(1)
            rv=re.search(r'revision:\s*(\d+)',s)
            if rv: member['rev']=int(rv.group(1))
        elif comp is not None and depth==2:
            nm=re.match(r'name:\s*"([^"]+)"',s)
            if nm: comp['name']=nm.group(1)
            pm=re.match(r'prototype:\s*"([^"]+)"',s)
            if pm: comp['proto']=pm.group(1)
            ex=re.findall(r'"([^"/]+(?:/[^"]+)?)\s+(\d+\.\d+)"',s)
            if 'exportMetaObjectRevisions' in s:
                comp['revs']+= [int(x) for x in re.findall(r'\d+',s.split(':',1)[1])]
        # exports multi-line
        if comp is not None and depth>=2 and re.match(r'^"[^"]+ \d+\.\d+",?$',s) :
            e=re.match(r'^"([^"]+) (\d+\.\d+)"',s); comp['exports'].append((e.group(1),e.group(2)))
        elif comp is not None and depth==2 and s.startswith('exports:') and '"' in s:
            for e in re.findall(r'"([^"]+) (\d+\.\d+)"',s): comp['exports'].append(e)
        depth+=ln.count('{')-ln.count('}')
        if depth<=2: member=None
        if depth<=1: comp=None
    return comps
if __name__=='__main__':
    comps=[]
    for p in sys.argv[1:]: comps+=parse(p)
    json.dump(comps,sys.stdout)
