#!/usr/bin/env python3
"""Every QML assignment and handler against the platform's own type data.

Qt 5.6 on Sailfish: a property, handler or import revision the target does not
have makes the page unloadable at run time, and the build says nothing. Reads
the target's plugins.qmltypes and Silica's QML, checks each object in qml/.
Usage: check.py <target qml dir> <app qml dir>. Exit 1 on any finding.
"""
import re,sys,os,glob,json
sys.path.insert(0,os.path.dirname(__file__))
from qmltypes import parse as parse_qmltypes

QMLROOT=sys.argv[1]   # target qml dir
APP=sys.argv[2]       # app qml dir
# ---------- type DB ----------
cpp={}      # class name -> comp
exports={}  # (module, Type) -> list of (ver, revision, classname)
for f in glob.glob(QMLROOT+'/**/*.qmltypes',recursive=True)+[QMLROOT+'/builtins.qmltypes']:
    for c in parse_qmltypes(f):
        if not c['name']: continue
        cpp.setdefault(c['name'],c)
        revs=c['revs'] if len(c['revs'])==len(c['exports']) else [0]*len(c['exports'])
        for (e,v),r in zip(c['exports'],revs):
            if '/' not in e: continue
            mod,t=e.rsplit('/',1)
            exports.setdefault((mod,t),[]).append((tuple(map(int,v.split('.'))),r,c['name']))

# ---------- tokenizer ----------
TOK=re.compile(r'''(?P<ws>[ \t\r]+)|(?P<nl>\n)|(?P<lc>//[^\n]*)|(?P<bc>/\*.*?\*/)|(?P<str>"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'|`(?:\\.|[^`\\])*`)|(?P<num>\d[\w.]*)|(?P<id>[A-Za-z_$][\w$]*)|(?P<op>[{}()\[\];,:?.]|[^\s\w])''',re.S)
RX=re.compile(r'/(?![*/])(?:\\.|\[(?:\\.|[^\]\\\n])*\]|[^/\\\n\[])+/[gimuy]*')
def tokenize(src):
    toks=[];line=1;nlb=False;pos=0
    while pos<len(src):
        if src[pos]=='/' and src[pos+1:pos+2] not in('/','*'):
            prev=toks[-1][1] if toks else ''
            pk=toks[-1][0] if toks else ''
            if pk not in('id','num','str') or prev in('return','typeof','case'):
                if prev not in(')',']','}') or pk=='op' and prev in('}',):
                    m=RX.match(src,pos)
                    if m and prev not in(')',']'):
                        toks.append(('str',m.group(),line,nlb));nlb=False;pos=m.end();continue
        m=TOK.match(src,pos)
        k=m.lastgroup;v=m.group();pos=m.end()
        if k=='nl' : line+=1;nlb=True;continue
        if k in('ws','lc'): continue
        if k=='bc':
            if '\n' in v: nlb=True
            line+=v.count('\n');continue
        toks.append((k,v,line,nlb));nlb=False
        if k=='str': line+=v.count('\n')
    toks.append(('eof','',line,True))
    return toks

OPEN={'(':')','[':']','{':'}'}
CONT_START=set('? : . + - * / % & | = < > , ^ !'.split())|{'&&','||'}
class P:
    def __init__(s,toks,fn): s.t=toks;s.i=0;s.fn=fn
    def peek(s,k=0): return s.t[min(s.i+k,len(s.t)-1)]
    def next(s): x=s.t[s.i]; s.i+=1; return x
    def skip_balanced(s):
        # current token is an opener
        stack=[]
        while True:
            k,v,l,n=s.next()
            if k=='eof': return
            if v in OPEN: stack.append(OPEN[v])
            elif stack and v==stack[-1]:
                stack.pop()
                if not stack: return
    def skip_expr(s):
        prev=None
        while True:
            k,v,l,n=s.peek()
            if k=='eof': return
            if v in (';',): s.next(); return
            if v=='}' : return
            if prev is not None and n and v not in CONT_START and prev not in CONT_START and prev not in ('return',) and not (prev in ('else',) ) :
                return
            if v in OPEN: s.skip_balanced(); prev=OPEN[v]; continue
            s.next(); prev=v
    def qname(s):
        k,v,l,n=s.next(); parts=[v]
        while s.peek()[1]=='.' and s.peek(1)[0]=='id':
            s.next(); parts.append(s.next()[1])
        return '.'.join(parts),l
    def is_obj_start(s):
        # Qualified.Type {
        j=s.i
        if s.t[j][0]!='id': return False
        last=s.t[j][1]
        while s.t[j+1][1]=='.' and s.t[j+2][0]=='id': j+=2; last=s.t[j][1]
        if not last[:1].isupper(): return False
        return s.t[j+1][1]=='{' or (s.t[j+1][1]=='on' and s.t[j+2][0]=='id')
    def obj(s):
        tname,line=s.qname()
        on=None
        if s.peek()[1]=='on':
            s.next(); on=s.qname()[0]
        assert s.next()[1]=='{', (s.fn,line)
        o={'type':tname,'line':line,'on':on,'assign':[],'decl':{},'children':[],'id':None}
        while True:
            k,v,l,n=s.peek()
            if v=='}': s.next(); return o
            if k=='eof': return o
            if v==';' or v==',': s.next(); continue
            if v in('property','readonly','default') and s.peek(1)[0]=='id' and (s.peek(2)[0]=='id' or s.peek(2)[1]=='<' or v!='property'):
                while s.peek()[1] in ('readonly','default'): s.next()
                s.next() # property
                typ=s.next()[1]
                if s.peek()[1]=='<':
                    while s.next()[1]!='>': pass
                nm=s.next()[1]; o['decl'][nm]=('property',l,typ)
                if s.peek()[1]==':':
                    s.next(); s.value(o,nm,l)
                continue
            if v=='signal':
                s.next(); nm=s.next()[1]; o['decl'][nm]=('signal',l,None)
                if s.peek()[1]=='(': s.skip_balanced()
                continue
            if v=='function':
                s.next(); nm=s.next()[1]; o['decl'][nm]=('function',l,None)
                s.skip_balanced() # params
                s.skip_balanced() # body
                continue
            if v=='enum': s.next(); s.next(); s.skip_balanced(); continue
            if s.is_obj_start():
                o['children'].append(s.obj()); continue
            if k=='id':
                nm,l2=s.qname()
                if s.peek()[1]==':':
                    s.next()
                    if nm=='id': o['id']=s.next()[1]; continue
                    o['assign'].append((nm,l2))
                    s.value(o,nm,l2)
                    continue
                # garbage: skip expr
                s.skip_expr(); continue
            s.next()
    def value(s,o,nm,l):
        if s.is_obj_start():
            c=s.obj(); c['asprop']=nm; o['children'].append(c); return
        if s.peek()[1]=='[':
            # list of objects?
            j=s.i+1
            if s.t[j][0]=='id':
                save=s.i; s.i=j
                if s.is_obj_start():
                    while True:
                        c=s.obj(); c['asprop']=nm; o['children'].append(c)
                        if s.peek()[1]==',': s.next(); continue
                        break
                    if s.peek()[1]==']': s.next()
                    return
                s.i=save
        if s.peek()[1]=='{':
            s.skip_balanced(); return
        s.skip_expr()

def parse_file(fn):
    src=open(fn,encoding='utf-8').read()
    toks=tokenize(src)
    imports=[];i=0
    p=P(toks,fn)
    while p.peek()[1]=='import' or p.peek()[1]=='.':
        p.next()
        k,v,l,n=p.next()
        if k=='str':
            alias=None
            if p.peek()[1]=='as': p.next(); alias=p.next()[1]
            imports.append(('file',v.strip('"'),None,alias))
        else:
            mod=v
            while p.peek()[1]=='.': p.next(); mod+='.'+p.next()[1]
            ver=p.next()[1]
            alias=None
            if p.peek()[1]=='as': p.next(); alias=p.next()[1]
            imports.append(('mod',mod,tuple(map(int,ver.split('.'))) if ver[0].isdigit() else None,alias))
        if p.peek()[1]==';': p.next()
    # pragma
    root=p.obj() if p.is_obj_start() else None
    return imports,root

# ---------- type resolution ----------
composite_cache={}
def module_dir(mod): return os.path.join(QMLROOT,*mod.split('.'))
def qmldir_types(mod):
    d=module_dir(mod); res={}
    q=os.path.join(d,'qmldir')
    if os.path.exists(q):
        for ln in open(q):
            m=re.match(r'^(?:singleton\s+)?([A-Z]\w*)\s+(\d+\.\d+)\s+(\S+\.qml)',ln.strip())
            if m: res.setdefault(m.group(1),[]).append((tuple(map(int,m.group(2).split('.'))),os.path.join(d,m.group(3))))
    return res
qd_cache={}
def resolve(tname,imports,fdir):
    """returns ('cpp',classname,rev,module) or ('qml',path) or None"""
    alias=None
    if '.' in tname: alias,t=tname.rsplit('.',1)
    else: t=tname
    cands=[]
    for kind,mod,ver,al in imports:
        if al!=alias: continue
        if kind=='file':
            d=os.path.normpath(os.path.join(fdir,mod))
            if os.path.isdir(d) and os.path.exists(os.path.join(d,t+'.qml')): return ('qml',os.path.join(d,t+'.qml'))
            q=os.path.join(d,'qmldir')
            if os.path.exists(q):
                mm=[l.split()[1] for l in open(q) if l.startswith('module ')]
                for ln in open(q):
                    m=re.match(r'^(?:singleton\s+)?([A-Z]\w*)\s+(\d+\.\d+)\s+(\S+\.qml)',ln.strip())
                    if m and m.group(1)==t: return ('qml',os.path.join(d,m.group(3)))
                if mm:
                    r=resolve(t,[('mod',mm[0],(1,0),None)],fdir)
                    if r: return r
            continue
        mods=[mod]+(['QtQml'] if mod=='QtQuick' else [])
        for mm in mods:
            if mm not in qd_cache: qd_cache[mm]=qmldir_types(mm)
            if t in qd_cache[mm]:
                ok=[x for x in qd_cache[mm][t] if x[0]<=(ver if mm==mod else (2,99))]
                if ok: return ('qml',max(ok)[1])
            ex=exports.get((mm,t))
            if ex:
                v=ver if mm==mod else (2,(ver[1] if ver else 0))
                ok=[x for x in ex if x[0][0]==v[0] and x[0]<=v]
                if ok:
                    best=max(ok); return ('cpp',best[2],best[1],mm,v)
                else:
                    return ('toonew',t,mm,min(ex)[0],v)
    if alias is None:
        if os.path.exists(os.path.join(fdir,t+'.qml')): return ('qml',os.path.join(fdir,t+'.qml'))
    return None

def cpp_members(cls,module,ver):
    """members allowed: dict name->kind; plus hidden dict name->(needed)"""
    allowed={};hidden={}
    c=cpp.get(cls)
    while c:
        # allowed revision for this class in module at ver
        rv=0
        revs=c['revs'] if len(c['revs'])==len(c['exports']) else [0]*len(c['exports'])
        for (e,v),r in zip(c['exports'],revs):
            if '/' in e and e.rsplit('/',1)[0] in (module,) and tuple(map(int,v.split('.')))<=ver: rv=max(rv,r)
        for m in c['members']:
            if not m['name']: continue
            if m['rev']<=rv: allowed.setdefault(m['name'],m['kind'])
            else: hidden.setdefault(m['name'],(c['name'],m['rev']))
        c=cpp.get(c['proto']) if c['proto'] else None
    return allowed,hidden

def members_of(res,depth=0):
    if res is None or depth>20: return None
    if res[0]=='cpp': return cpp_members(res[1],res[3],res[4])
    if res[0]=='qml':
        path=res[1]
        if path in composite_cache: return composite_cache[path]
        composite_cache[path]=({},{})
        try: imps,root=parse_file(path)
        except Exception as e: return None
        if root is None: return None
        base=members_of(resolve(root['type'],imps,os.path.dirname(path)),depth+1)
        allowed=dict(base[0]) if base else {'__unknown__':1}
        hidden=dict(base[1]) if base else {}
        for nm,(kind,l,typ) in root['decl'].items():
            allowed[nm]=kind
            if kind=='property': allowed[nm+'Changed']='Signal'
        composite_cache[path]=(allowed,hidden)
        return allowed,hidden
    return None

def handler_ok(nm,allowed):
    if not re.match(r'^on[A-Z]',nm): return None
    sig=nm[2].lower()+nm[3:]
    if sig in allowed: return True
    if sig.endswith('Changed') and sig[:-7] in allowed: return True
    return False

findings=[]
stats={'checked':0,'skipped':0,'assign':0}
skipped_types={}
def check_obj(o,imps,fdir,fn,inherited_unknown=False):
    t=o['type']
    res=resolve(t,imps,fdir)
    if res is None:
        findings.append((fn,o['line'],'UNRESOLVED type '+t)); mem=None
    elif res[0]=='toonew':
        findings.append((fn,o['line'],'TYPE %s needs %s %s, imported %s'%(t,res[2],'.'.join(map(str,res[3])),res[4]))); mem=None
    else:
        mem=members_of(res)
    if not (mem and '__unknown__' not in mem[0] and t not in ('Connections','PropertyChanges','ListElement','Binding')):
        stats['skipped']+=1; skipped_types[t]=skipped_types.get(t,0)+1
    else:
        stats['checked']+=1; stats['assign']+=len(o['assign'])
    if mem and '__unknown__' not in mem[0] and t not in ('Connections','PropertyChanges','ListElement','Binding'):
        allowed,hidden=mem
        decl=o['decl']
        for nm,l in o['assign']:
            first=nm.split('.')[0]
            if first[:1].isupper(): continue  # attached
            if first in decl: continue
            if re.match(r'^on[A-Z]',first):
                sg=first[2].lower()+first[3:]
                if sg in decl or (sg.endswith('Changed') and sg[:-7] in decl): continue
            h=handler_ok(first,allowed) if '.' not in nm else None
            if h is True: continue
            if h is False:
                sig=first[2].lower()+first[3:]
                why=hidden.get(sig) or (hidden.get(sig[:-7]) if sig.endswith('Changed') else None)
                findings.append((fn,l,'HANDLER %s not on %s%s'%(first,t,(' (hidden: needs revision %s of %s)'%(why[1],why[0])) if why else '')))
                continue
            if first in allowed: continue
            why=hidden.get(first)
            findings.append((fn,l,'PROPERTY %s not on %s%s'%(first,t,(' (hidden: needs revision %s of %s)'%(why[1],why[0])) if why else '')))
        for nm,(kind,l,typ) in decl.items():
            pass
    for c in o['children']: check_obj(c,imps,fdir,fn)
    # duplicate assignments
    seen={}
    for nm,l in o['assign']:
        if nm in seen: findings.append((fn,l,'DUPLICATE %s (also line %d) in %s'%(nm,seen[nm],t)))
        else: seen[nm]=l

files=sorted(glob.glob(APP+'/**/*.qml',recursive=True))
for fn in files:
    try:
        imps,root=parse_file(fn)
    except Exception as e:
        findings.append((fn,0,'PARSE ERROR %r'%(e,))); continue
    for kind,mod,ver,al in imps:
        if kind=='mod' and mod=='QtQuick' and ver and ver>(2,6): findings.append((fn,0,'IMPORT QtQuick %s > 2.6'%(ver,)))
    if root: check_obj(root,imps,os.path.dirname(fn),fn)
for f in findings: print('%s:%s: %s'%(os.path.relpath(f[0],APP),f[1],f[2]))
print(stats,skipped_types,file=sys.stderr)
print('files',len(files),'findings',len(findings),file=sys.stderr)
sys.exit(1 if findings else 0)
