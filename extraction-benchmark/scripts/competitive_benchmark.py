#!/usr/bin/env python3
"""Competitive extraction benchmark for capped synthetic corpus runs.

Measurement only: runs Wellfriend and installable competitors in isolated
subprocesses, scores outputs against paired JSON ground truth, writes raw JSONL
under target/competitive-benchmark, and renders docs/competitive_benchmark.md.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import ctypes
import difflib
import hashlib
import json
import math
import os
import re
import shutil
import signal
import statistics
import subprocess
import sys
import time
import zipfile
from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

REPO = Path(__file__).resolve().parents[2]
DEFAULT_CORPUS = REPO / "test_corpus"
DEFAULT_OUTPUT = REPO / "target" / "competitive-benchmark" / "latest"
DEFAULT_REPORT = REPO / "docs" / "competitive_benchmark.md"
PUBLIC_MATRIX = REPO / "public-benchmark" / "capability_matrix.json"

PY_PYMUPDF_TEXT = r'''
import sys, fitz
parts=[]
doc=fitz.open(sys.argv[1])
try:
    for p in doc: parts.append(p.get_text("text") or "")
finally:
    doc.close()
open(sys.argv[2],"w",encoding="utf-8",errors="replace").write("\n".join(parts))
'''
PY_PYPDFIUM2_TEXT = r'''
import sys, pypdfium2 as pdfium
parts=[]
pdf=pdfium.PdfDocument(sys.argv[1])
try:
    for i in range(len(pdf)):
        page=pdf[i]; tp=page.get_textpage()
        try: parts.append(tp.get_text_range() or "")
        finally: tp.close(); page.close()
finally:
    pdf.close()
open(sys.argv[2],"w",encoding="utf-8",errors="replace").write("\n".join(parts))
'''
PY_PDFMINER_TEXT = r'''
import sys
from pdfminer.high_level import extract_text
open(sys.argv[2],"w",encoding="utf-8",errors="replace").write(extract_text(sys.argv[1]) or "")
'''
PY_PDFPLUMBER_TEXT = r'''
import sys, pdfplumber
parts=[]
with pdfplumber.open(sys.argv[1]) as pdf:
    for p in pdf.pages: parts.append(p.extract_text() or "")
open(sys.argv[2],"w",encoding="utf-8",errors="replace").write("\n".join(parts))
'''
PY_PYPDF_TEXT = r'''
import sys
from pypdf import PdfReader
r=PdfReader(sys.argv[1])
if getattr(r,"is_encrypted",False):
    try: r.decrypt("")
    except Exception: pass
open(sys.argv[2],"w",encoding="utf-8",errors="replace").write("\n".join((p.extract_text() or "") for p in r.pages))
'''
PY_PYMUPDF4LLM_TEXT = r'''
import sys, pymupdf4llm
try: text=pymupdf4llm.to_text(sys.argv[1], use_ocr=False)
except Exception: text=pymupdf4llm.to_markdown(sys.argv[1])
open(sys.argv[2],"w",encoding="utf-8",errors="replace").write(text or "")
'''
PY_MARKITDOWN_TEXT = r'''
import sys
from markitdown import MarkItDown
r=MarkItDown(enable_plugins=False).convert(sys.argv[1])
open(sys.argv[2],"w",encoding="utf-8",errors="replace").write(getattr(r,"text_content","") or "")
'''
PY_PDFTEXT_TEXT = r'''
import sys
path=sys.argv[1]; text=None; errors=[]
try:
    from pdftext.extraction import plain_text_output
    v=plain_text_output(path)
    if isinstance(v,str): text=v
    elif isinstance(v,(list,tuple)): text="\n".join(str(x) for x in v)
    elif isinstance(v,dict): text="\n".join(str(x) for x in v.values())
except Exception as e: errors.append(str(e))
if text is None:
    try:
        from pdftext.extraction import dictionary_output
        v=dictionary_output(path); pages=v if isinstance(v,list) else v.get("pages",[]); chunks=[]
        for page in pages:
            for b in page.get("blocks",[]):
                if "text" in b: chunks.append(str(b["text"]))
                for line in b.get("lines",[]): chunks.append(" ".join(str(s.get("text","")) for s in line.get("spans",[])))
        text="\n".join(chunks)
    except Exception as e: errors.append(str(e))
if text is None: raise RuntimeError("; ".join(errors) or "pdftext failed")
open(sys.argv[2],"w",encoding="utf-8",errors="replace").write(text or "")
'''
PY_PDF_WELLFRIENDPDF_TEXT = r'''
import sys
from pdf_wellfriendpdf import PdfDocument
doc=PdfDocument(sys.argv[1]); parts=[]
try:
    pc=getattr(doc,"page_count",None); n=pc() if callable(pc) else int(pc)
except Exception: n=len(doc)
for i in range(n):
    if hasattr(doc,"extract_text"): parts.append(doc.extract_text(i) or "")
    else:
        p=doc[i]; t=getattr(p,"text",""); parts.append(t() if callable(t) else t or "")
open(sys.argv[2],"w",encoding="utf-8",errors="replace").write("\n".join(parts))
'''
PY_DOCLING_TEXT = r'''
import os, sys
if os.environ.get("BENCH_DOCLING_NO_OCR")=="1":
    from docling.datamodel.base_models import InputFormat
    from docling.datamodel.pipeline_options import PdfPipelineOptions
    from docling.document_converter import DocumentConverter, PdfFormatOption
    opts=PdfPipelineOptions(do_ocr=False)
    converter=DocumentConverter(format_options={InputFormat.PDF: PdfFormatOption(pipeline_options=opts)})
else:
    from docling.document_converter import DocumentConverter
    converter=DocumentConverter()
r=converter.convert(sys.argv[1]); d=r.document
text=d.export_to_markdown() if hasattr(d,"export_to_markdown") else (d.export_to_text() if hasattr(d,"export_to_text") else str(d))
open(sys.argv[2],"w",encoding="utf-8",errors="replace").write(text or "")
'''
PY_PYMUPDF_TABLES = r'''
import json, sys, fitz
out=[]; doc=fitz.open(sys.argv[1])
try:
    for p in doc:
        try: tables=getattr(p.find_tables(),"tables",[]) or []
        except Exception: tables=[]
        for t in tables: out.append({"rows":[["" if c is None else str(c) for c in row] for row in (t.extract() or [])]})
finally: doc.close()
json.dump({"tables":out},open(sys.argv[2],"w",encoding="utf-8"),ensure_ascii=False)
'''
PY_PDFPLUMBER_TABLES = r'''
import json, sys, pdfplumber
out=[]
with pdfplumber.open(sys.argv[1]) as pdf:
    for p in pdf.pages:
        for rows in (p.extract_tables() or []): out.append({"rows":[["" if c is None else str(c) for c in row] for row in (rows or [])]})
json.dump({"tables":out},open(sys.argv[2],"w",encoding="utf-8"),ensure_ascii=False)
'''
PY_PYPDF_FIELDS = r'''
import json, sys
from pypdf import PdfReader
r=PdfReader(sys.argv[1]); out=[]
try: fields=r.get_fields() or {}
except Exception: fields={}
for k,v in fields.items():
    val=v.get('/V',v.get('/DV','')) if isinstance(v,dict) else v
    out.append({"key":str(k),"raw":str(val)})
json.dump({"fields":out},open(sys.argv[2],"w",encoding="utf-8"),ensure_ascii=False)
'''
PY_PYMUPDF_IMAGES = r'''
import json, sys, fitz
n=0; doc=fitz.open(sys.argv[1])
try:
    for p in doc: n += len(p.get_images(full=True) or [])
finally: doc.close()
json.dump({"image_count":n},open(sys.argv[2],"w",encoding="utf-8"))
'''
PY_PDFPLUMBER_IMAGES = r'''
import json, sys, pdfplumber
n=0
with pdfplumber.open(sys.argv[1]) as pdf:
    for p in pdf.pages: n += len(p.images or [])
json.dump({"image_count":n},open(sys.argv[2],"w",encoding="utf-8"))
'''
PY_PYPDF_IMAGES = r'''
import json, sys
from pypdf import PdfReader
r=PdfReader(sys.argv[1]); seen=set()
def obj(x):
    try: return x.get_object()
    except Exception: return x
def walk(xo):
    total=0; d=obj(xo)
    if not isinstance(d,dict): return 0
    for _name,ch in d.items():
        c=obj(ch)
        try: sub=str(c.get('/Subtype'))
        except Exception: sub=''
        if sub=='/Image': total+=1
        elif sub=='/Form' and id(c) not in seen:
            seen.add(id(c)); res=obj(c.get('/Resources',{})) if hasattr(c,'get') else {}; total += walk(res.get('/XObject',{}) if hasattr(res,'get') else {})
    return total
n=0
for p in r.pages:
    res=obj(p.get('/Resources',{})); n += walk(res.get('/XObject',{}) if hasattr(res,'get') else {})
json.dump({"image_count":n},open(sys.argv[2],"w",encoding="utf-8"))
'''

@dataclass
class Result:
    ok: bool; code: int|None; timeout: bool; mem_exceeded: bool; ms: int; peak_mb: float|None; out: str; err: str; error: str|None=None
    def kind(self):
        if self.ok: return None
        if self.timeout: return "timeout"
        if self.mem_exceeded: return "memory"
        if self.code is None: return "launch"
        return "error"

@dataclass
class Tool:
    name: str; kind: str; import_name: str|None; license: str; cmd: Callable[[Path,Path,argparse.Namespace],list[str]]; dist: str|None=None

def exe(name:str)->str: return name+".exe" if os.name=="nt" and not name.endswith(".exe") else name

def default_wellfriendpdf()->Path:
    rel=REPO/"target"/"release"/exe("wellfriendpdf"); dbg=REPO/"target"/"debug"/exe("wellfriendpdf")
    return rel if rel.exists() else dbg

def python_for(tool_name,args):
    if tool_name=="docling" and getattr(args,"docling_python",None):
        return str(Path(args.docling_python))
    return sys.executable
def pycmd(code,pdf,out,_args): return [sys.executable,"-c",code,str(pdf),str(out)]
def tool_pycmd(tool_name,code,pdf,out,args): return [python_for(tool_name,args),"-c",code,str(pdf),str(out)]
def wellfriendpdf_text(pdf,out,args): return [str(Path(args.wellfriendpdf_bin)),"extract-text",str(pdf),"--output",str(out)]
def wellfriendpdf_tables(pdf,out,args): return [str(Path(args.wellfriendpdf_bin)),"extract-tables",str(pdf),"--format","json","--output",str(out)]
def wellfriendpdf_fields(pdf,out,args): return [str(Path(args.wellfriendpdf_bin)),"extract-fields",str(pdf),"--format","json","--output",str(out)]
def wellfriendpdf_images(pdf,out,args): return [str(Path(args.wellfriendpdf_bin)),"extract-images",str(pdf),"--format","original","--output",str(out)]
def poppler_text(pdf,out,args): return ["pdftotext","-layout",str(pdf),str(out)]
def poppler_images(pdf,out,args): return ["pdfimages","-list",str(pdf)]

def text_tools(): return [
    Tool("wellfriendpdf","local",None,"MIT OR Apache-2.0",wellfriendpdf_text),
    Tool("pdf_wellfriendpdf","python","pdf_wellfriendpdf","MIT",lambda p,o,a:pycmd(PY_PDF_WELLFRIENDPDF_TEXT,p,o,a)),
    Tool("pymupdf","python","fitz","AGPL-3.0/commercial",lambda p,o,a:pycmd(PY_PYMUPDF_TEXT,p,o,a),"PyMuPDF"),
    Tool("pypdfium2","python","pypdfium2","Apache-2.0/BSD-3-Clause",lambda p,o,a:pycmd(PY_PYPDFIUM2_TEXT,p,o,a)),
    Tool("pdfminer.six","python","pdfminer","MIT",lambda p,o,a:pycmd(PY_PDFMINER_TEXT,p,o,a),"pdfminer.six"),
    Tool("pdfplumber","python","pdfplumber","MIT",lambda p,o,a:pycmd(PY_PDFPLUMBER_TEXT,p,o,a)),
    Tool("pypdf","python","pypdf","BSD-3-Clause",lambda p,o,a:pycmd(PY_PYPDF_TEXT,p,o,a)),
    Tool("pymupdf4llm","python","pymupdf4llm","AGPL-3.0/commercial",lambda p,o,a:pycmd(PY_PYMUPDF4LLM_TEXT,p,o,a)),
    Tool("pdftext","python","pdftext","Apache-2.0",lambda p,o,a:pycmd(PY_PDFTEXT_TEXT,p,o,a)),
    Tool("markitdown","python","markitdown","MIT",lambda p,o,a:pycmd(PY_MARKITDOWN_TEXT,p,o,a)),
    Tool("docling","python","docling","MIT",lambda p,o,a:tool_pycmd("docling",PY_DOCLING_TEXT,p,o,a)),
    Tool("poppler","cli",None,"GPL-2.0-or-later",poppler_text)]
def table_tools(): return [Tool("wellfriendpdf","local",None,"MIT OR Apache-2.0",wellfriendpdf_tables),Tool("pymupdf","python","fitz","AGPL-3.0/commercial",lambda p,o,a:pycmd(PY_PYMUPDF_TABLES,p,o,a),"PyMuPDF"),Tool("pdfplumber","python","pdfplumber","MIT",lambda p,o,a:pycmd(PY_PDFPLUMBER_TABLES,p,o,a))]
def field_tools(): return [Tool("wellfriendpdf","local",None,"MIT OR Apache-2.0",wellfriendpdf_fields),Tool("pypdf","python","pypdf","BSD-3-Clause",lambda p,o,a:pycmd(PY_PYPDF_FIELDS,p,o,a))]
def image_tools(): return [Tool("wellfriendpdf","local",None,"MIT OR Apache-2.0",wellfriendpdf_images),Tool("pymupdf","python","fitz","AGPL-3.0/commercial",lambda p,o,a:pycmd(PY_PYMUPDF_IMAGES,p,o,a),"PyMuPDF"),Tool("pdfplumber","python","pdfplumber","MIT",lambda p,o,a:pycmd(PY_PDFPLUMBER_IMAGES,p,o,a)),Tool("pypdf","python","pypdf","BSD-3-Clause",lambda p,o,a:pycmd(PY_PYPDF_IMAGES,p,o,a)),Tool("poppler","cli",None,"GPL-2.0-or-later",poppler_images)]

def cap(s,n=600): return "" if not s else (s if len(s)<=n else s[:n]+" ...")
def run_cap(cmd,timeout=20):
    try:
        p=subprocess.run(cmd,cwd=REPO,capture_output=True,text=True,encoding="utf-8",errors="replace",timeout=timeout)
        return p.returncode==0,(p.stdout or p.stderr or "").strip()
    except Exception as e: return False,str(e)

def detect_one(t,args):
    if t.name=="wellfriendpdf":
        p=Path(args.wellfriendpdf_bin); ok=p.exists(); ver=None
        if ok: _ok,out=run_cap([str(p),"--version"]); ver=out.splitlines()[0] if out else None
        return {"available":ok,"version":ver,"reason":None if ok else f"missing {p}","license":t.license}
    if t.name=="poppler":
        ok=shutil.which("pdftotext") and shutil.which("pdfinfo")
        ver=None
        if ok: _ok,out=run_cap(["pdftotext","-v"]); ver=out.splitlines()[0] if out else None
        return {"available":bool(ok),"version":ver,"reason":None if ok else "pdftotext/pdfinfo missing","license":t.license}
    if t.kind=="python":
        code=f"import importlib.metadata as m\nimport {t.import_name}\ntry: print(m.version({(t.dist or t.import_name)!r}))\nexcept Exception: print('import-ok/version-unknown')\n"
        interp=python_for(t.name,args)
        p=subprocess.run([interp,"-c",code],cwd=REPO,capture_output=True,encoding="utf-8",errors="replace",text=True)
        return {"available":p.returncode==0,"version":p.stdout.strip() if p.returncode==0 else None,"reason":None if p.returncode==0 else cap(p.stderr or p.stdout),"license":t.license}
    return {"available":False,"version":None,"reason":"unknown detector","license":t.license}

def detect_all(args):
    tools={}
    for group in (text_tools(),table_tools(),field_tools(),image_tools()):
        for t in group:
            if t.name not in tools: tools[t.name]=detect_one(t,args)
    ok,out=run_cap(["qpdf","--version"]); tools["qpdf"]={"available":ok,"version":out.splitlines()[0] if out else None,"reason":None if ok else out,"license":"Apache-2.0"}
    ok,out=run_cap(["pdftoppm","-v"]); tools["pdftoppm"]={"available":ok,"version":out.splitlines()[0] if out else None,"reason":None if ok else out,"license":"GPL-2.0-or-later"}
    return tools

def env(args=None):
    e=os.environ.copy(); e.update({"RAYON_NUM_THREADS":"1","OMP_NUM_THREADS":"1","MKL_NUM_THREADS":"1","OPENBLAS_NUM_THREADS":"1","NUMEXPR_NUM_THREADS":"1","TOKENIZERS_PARALLELISM":"false"})
    if args and getattr(args,"docling_no_ocr",False): e["BENCH_DOCLING_NO_OCR"]="1"
    return e

def kill_tree(p):
    if p.poll() is not None: return
    if os.name=="nt": subprocess.run(["taskkill","/PID",str(p.pid),"/T","/F"],capture_output=True,text=True)
    else:
        try: os.killpg(p.pid,signal.SIGKILL)
        except Exception: p.kill()

def rss(pid):
    if os.name!="nt":
        try:
            for line in Path(f"/proc/{pid}/status").read_text().splitlines():
                if line.startswith("VmRSS:"): return int(line.split()[1])/1024
        except Exception: return None
        return None
    class PMC(ctypes.Structure):
        _fields_=[("cb",ctypes.c_ulong),("PageFaultCount",ctypes.c_ulong),("PeakWorkingSetSize",ctypes.c_size_t),("WorkingSetSize",ctypes.c_size_t),("QuotaPeakPagedPoolUsage",ctypes.c_size_t),("QuotaPagedPoolUsage",ctypes.c_size_t),("QuotaPeakNonPagedPoolUsage",ctypes.c_size_t),("QuotaNonPagedPoolUsage",ctypes.c_size_t),("PagefileUsage",ctypes.c_size_t),("PeakPagefileUsage",ctypes.c_size_t)]
    try:
        k=ctypes.WinDLL("kernel32",use_last_error=True); ps=ctypes.WinDLL("psapi",use_last_error=True); h=k.OpenProcess(0x1000|0x0010,False,pid)
        if not h: return None
        c=PMC(); c.cb=ctypes.sizeof(c); ok=ps.GetProcessMemoryInfo(h,ctypes.byref(c),c.cb); k.CloseHandle(h)
        return c.WorkingSetSize/1048576 if ok else None
    except Exception: return None

RESOURCE_ERROR_MARKERS=("WinError 1450","Insufficient system resources")
TASK_NAMES=("text","tables","fields","images")

def monitored(cmd,args):
    start=time.monotonic(); peak=None; timeout=False; memex=False; err=None
    flags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name=="nt" else 0; pre=None if os.name=="nt" else os.setsid
    p=None
    try: p=subprocess.Popen(cmd,cwd=REPO,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,encoding="utf-8",errors="replace",env=env(args),creationflags=flags,preexec_fn=pre)
    except FileNotFoundError as e: return Result(False,None,False,False,0,None,"","",str(e))
    out=er=""
    try:
        while p.poll() is None:
            r=rss(p.pid)
            if r is not None:
                peak=r if peak is None else max(peak,r)
                if args.max_memory_mb and r>args.max_memory_mb: memex=True; err=f"memory cap {r:.1f}>{args.max_memory_mb}"; kill_tree(p); break
            if time.monotonic()-start>args.timeout: timeout=True; err=f"timeout after {args.timeout}s"; kill_tree(p); break
            try:
                time.sleep(max(0.01,args.poll_interval_ms/1000))
            except OSError as e:
                err=str(e); kill_tree(p); break
        try: out,er=p.communicate(timeout=5)
        except subprocess.TimeoutExpired: kill_tree(p); out,er=p.communicate()
    finally:
        for pipe in (getattr(p,"stdout",None),getattr(p,"stderr",None)):
            try:
                if pipe: pipe.close()
            except Exception: pass
        if p and p.poll() is None:
            kill_tree(p)
            try: p.wait(timeout=2)
            except Exception: pass
    ms=int(round((time.monotonic()-start)*1000)); ok=p.returncode==0 and not timeout and not memex and err is None
    return Result(ok,p.returncode,timeout,memex,ms,peak,out,er,err)

def norm_text(s): s=s.replace("\r\n","\n").replace("\r","\n"); s=re.sub(r"[ \t\f\v]+"," ",s); s=re.sub(r"\n{3,}","\n\n",s); return s.strip()
def norm_line(s): return re.sub(r"\s+"," ",str(s or "")).strip().lower()
def toks(s): return re.findall(r"\w+|[^\w\s]",norm_text(s).lower(),flags=re.UNICODE)
def dice(a,b):
    ca,cb=Counter(a),Counter(b); na,nb=sum(ca.values()),sum(cb.values())
    if not na and not nb: return 1.0
    if not na or not nb: return 0.0
    return 2*sum(min(v,cb[k]) for k,v in ca.items())/(na+nb)
def char_sim(a,b):
    a,b=norm_text(a),norm_text(b)
    if not a and not b: return 1.0
    if not a or not b: return 0.0
    return dice(toks(a),toks(b)) if max(len(a),len(b))>25000 else difflib.SequenceMatcher(None,a,b,autojunk=True).ratio()
def text_score(lines,cand):
    ref="\n".join(lines); cand=norm_text(cand); gt=[norm_line(x) for x in lines if norm_line(x)]; cl=[norm_line(x) for x in cand.splitlines() if norm_line(x)]; joined="\n".join(cl); refj="\n".join(gt)
    pos=[]; hits=0
    for line in gt:
        p=joined.find(line); pos.append(None if p<0 else p); hits += 1 if p>=0 else 0
    found=[p for p in pos if p is not None]
    if len(found)<2: order=1.0 if len(gt)<=1 and hits==len(gt) else (0.0 if gt else None)
    else:
        inv=tot=0
        for i in range(len(found)):
            for j in range(i+1,len(found)): tot+=1; inv += 1 if found[i]>found[j] else 0
        order=1-inv/tot if tot else 1.0
    spur=sum(1 for x in cl if x and x not in refj)
    return {"char_similarity":round(char_sim(ref,cand),5),"word_f1":round(dice(toks(ref),toks(cand)),5),"line_recall":round(hits/len(gt),5) if gt else None,"spurious_line_ratio":round(spur/len(cl),5) if cl else (0.0 if not gt else 1.0),"reading_order":round(order,5) if isinstance(order,float) else order,"matched_lines":hits,"gt_lines":len(gt),"candidate_lines":len(cl)}
def cells(grids):
    c=Counter()
    for g in grids:
        for r in g or []:
            for cell in r or []:
                n=norm_line(cell)
                if n: c[n]+=1
    return c
def shapes(grids): return Counter(f"{len(g or [])}x{max((len(r or []) for r in g or []),default=0)}" for g in grids if g)
def prf(tp,pred,truth):
    p=tp/pred if pred else (1.0 if truth==0 else 0.0); r=tp/truth if truth else (1.0 if pred==0 else 0.0); f=2*p*r/(p+r) if p+r else 0.0; return p,r,f
def table_score(truth,pred):
    tg=[([t.get("headers") or []] if t.get("headers") else []) + (t.get("rows") or []) for t in truth]
    pg=[t.get("rows") or [] for t in pred]
    tc,pc=cells(tg),cells(pg); tp=sum(min(v,pc[k]) for k,v in tc.items()); p,r,f=prf(tp,sum(pc.values()),sum(tc.values()))
    ts,ps=shapes(tg),shapes(pg); stp=sum(min(v,ps[k]) for k,v in ts.items()); _p,_r,sf=prf(stp,sum(ps.values()),sum(ts.values()))
    return {"table_cell_precision":round(p,5),"table_cell_recall":round(r,5),"table_cell_f1":round(f,5),"table_shape_f1":round(sf,5),"table_teds_approx":round(.75*f+.25*sf,5),"truth_tables":len(tg),"predicted_tables":len(pg),"truth_cells":sum(tc.values()),"predicted_cells":sum(pc.values())}
def nkey(k): return re.sub(r"[^a-z0-9]+","_",str(k or "").lower()).strip("_")
def nval(v):
    if isinstance(v,dict):
        for k in ("iso","text","amount","number","value"):
            if k in v: return nval(v[k])
    return norm_line(v)
def pred_fields(payload):
    out=[]
    for f in payload.get("fields") or []:
        if isinstance(f,dict):
            val=f.get("raw") if f.get("raw") not in (None,"") else f.get("value"); out.append((nkey(f.get("key")),nval(val)))
    return [(k,v) for k,v in out if k or v]
def field_score(truth,payload):
    t=[(nkey(k),nval(v)) for k,v in truth.items() if nval(v)]; p=pred_fields(payload)
    tc=Counter([k+"\0"+v for k,v in t]); pc=Counter([k+"\0"+v for k,v in p]); tp=sum(min(v,pc[k]) for k,v in tc.items()); pr,rr,ff=prf(tp,sum(pc.values()),sum(tc.values()))
    tv,pv=Counter(v for _k,v in t),Counter(v for _k,v in p); vtp=sum(min(v,pv[k]) for k,v in tv.items()); vp,vr,vf=prf(vtp,sum(pv.values()),sum(tv.values()))
    return {"field_precision":round(pr,5),"field_recall":round(rr,5),"field_f1":round(ff,5),"field_value_precision":round(vp,5),"field_value_recall":round(vr,5),"field_value_f1":round(vf,5),"truth_fields":len(t),"predicted_fields":len(p)}
def image_score(truth,pred):
    if pred is None: return {"image_count_accuracy":None,"image_count_error":None,"image_exact":False}
    d=abs(int(pred)-int(truth)); return {"predicted_images":int(pred),"truth_images":int(truth),"image_count_error":d,"image_count_accuracy":round(1-d/max(int(pred),int(truth),1),5),"image_exact":d==0}

def tags(label,pdf):
    out=[label.get("kind") or "digital-text"]; pages=int(label.get("page_count") or 0); imgs=int(label.get("images") or 0); text_len=sum(len(str(x)) for x in label.get("text") or [])
    out.append("short-doc" if pages<=3 else ("long-doc" if pages>=20 else "medium-doc"))
    if label.get("tables"): out.append("has-tables")
    if label.get("fields"): out.append("has-fields")
    if imgs: out.append("has-images")
    if imgs>=10: out.append("image-heavy-count")
    if text_len>=10000: out.append("text-heavy")
    if pdf.stat().st_size>=5*1024*1024: out.append("large-file")
    if any(ord(ch)>127 for ch in "\n".join(label.get("text") or [])): out.append("multilingual-or-nonascii")
    return out
def load_entries(corpus,limit,category):
    corpus=Path(corpus).resolve()
    out=[]
    for jp in sorted(corpus.glob("*.json")):
        lab=json.loads(jp.read_text(encoding="utf-8")); pdf=Path(corpus)/(lab.get("file") or jp.with_suffix(".pdf").name)
        if not pdf.exists(): continue
        pdf=pdf.resolve()
        rec={"id":pdf.stem,"pdf":pdf,"json":jp.resolve(),"label":lab,"tags":tags(lab,pdf)}
        if category and category not in rec["tags"]: continue
        out.append(rec)
        if limit and len(out)>=limit: break
    return out
def corpus_summary(entries):
    pages=[int(e["label"].get("page_count") or 0) for e in entries]; imgs=[int(e["label"].get("images") or 0) for e in entries]; tagc=Counter(t for e in entries for t in e["tags"])
    return {"files":len(entries),"pages":sum(pages),"min_pages":min(pages) if pages else 0,"max_pages":max(pages) if pages else 0,"images":sum(imgs),"tags":dict(sorted(tagc.items()))}
def wanted(args): return {x.strip() for x in args.tools.split(",") if x.strip()} if args.tools else None
def avail(group,availability,args):
    w=wanted(args); return [t for t in group if (not w or t.name in w) and availability.get(t.name,{}).get("available")]
def base(entry):
    l=entry["label"]
    return {"id":entry["id"],"path":str(entry["pdf"].resolve().relative_to(REPO)),"tags":entry["tags"],"page_count":l.get("page_count"),"truth_text_lines":len(l.get("text") or []),"truth_tables":len(l.get("tables") or []),"truth_fields":len(l.get("fields") or {}),"truth_images":int(l.get("images") or 0),"size_bytes":entry["pdf"].stat().st_size,"text":{},"tables":{},"fields":{},"images":{}}
def rec_result(r): return {"ok":r.ok,"duration_ms":r.ms,"peak_memory_mb":r.peak_mb,"failure_kind":r.kind(),"failure":None if r.ok else {"exit_code":r.code,"timeout":r.timeout,"memory":r.mem_exceeded,"stdout":cap(r.out),"stderr":cap(r.err),"error":r.error}}
def unlink(p):
    try:
        if p.exists(): p.unlink()
    except OSError: pass
def read_json(p):
    try: return json.loads(p.read_text(encoding="utf-8",errors="replace"))
    except Exception: return None
def run_text(entry,t,args,work):
    out=work/(t.name+".txt"); r=monitored(t.cmd(entry["pdf"],out,args),args); rec=rec_result(r)
    if r.ok and out.exists():
        text=norm_text(out.read_text(encoding="utf-8",errors="replace")); rec.update(text_score(entry["label"].get("text") or [],text)); rec["text_chars"]=len(text); rec["text_sha256"]=hashlib.sha256(text.encode()).hexdigest()
    elif r.ok: rec.update({"ok":False,"failure_kind":"missing-output"})
    unlink(out); return rec
def run_tables(entry,t,args,work):
    out=work/(t.name+".tables.json"); r=monitored(t.cmd(entry["pdf"],out,args),args); rec=rec_result(r); payload=read_json(out) if r.ok and out.exists() else None
    if r.ok and payload is not None:
        pred=[]
        if t.name=="wellfriendpdf":
            for p in payload.get("pages",[]):
                for tb in p.get("tables",[]): pred.append({"rows":tb.get("rows") or []})
        else: pred=payload.get("tables") or []
        rec.update(table_score(entry["label"].get("tables") or [],pred))
    elif r.ok: rec.update({"ok":False,"failure_kind":"bad-json"})
    unlink(out); return rec
def run_fields(entry,t,args,work):
    out=work/(t.name+".fields.json"); r=monitored(t.cmd(entry["pdf"],out,args),args); rec=rec_result(r); payload=read_json(out) if r.ok and out.exists() else None
    if r.ok and payload is not None: rec.update(field_score(entry["label"].get("fields") or {},payload))
    elif r.ok: rec.update({"ok":False,"failure_kind":"bad-json"})
    unlink(out); return rec
def run_images(entry,t,args,work):
    out=work/(t.name+(".images.zip" if t.name=="wellfriendpdf" else ".images.json")); r=monitored(t.cmd(entry["pdf"],out,args),args); rec=rec_result(r); pred=None
    if r.ok:
        if t.name=="wellfriendpdf":
            try:
                with zipfile.ZipFile(out) as z: pred=len([n for n in z.namelist() if not n.endswith("/")])
            except Exception: rec.update({"ok":False,"failure_kind":"bad-zip"})
        elif t.name=="poppler": pred=max(0,len([ln for ln in r.out.splitlines() if ln.strip()])-2)
        else:
            payload=read_json(out); pred=payload.get("image_count") if payload else None
            if payload is None: rec.update({"ok":False,"failure_kind":"bad-json"})
    if rec.get("ok"): rec.update(image_score(int(entry["label"].get("images") or 0),pred))
    unlink(out); return rec
def sanitize(s): return re.sub(r"[^A-Za-z0-9_.-]+","_",s)[:160]
def merge_record(old,new):
    if old is None: return new
    merged=dict(old)
    for k,v in new.items():
        if k in TASK_NAMES:
            bucket=dict(merged.get(k) or {})
            bucket.update(v or {})
            merged[k]=bucket
        else:
            merged[k]=v
    for k in TASK_NAMES: merged.setdefault(k,{})
    return merged
def requested_tasks(args):
    if args.tasks: names=[x.strip().lower() for x in args.tasks.split(",") if x.strip()]
    else: names=["text","tables","fields"]
    bad=[x for x in names if x not in TASK_NAMES]
    if bad: raise SystemExit(f"unknown task(s): {', '.join(bad)}")
    if args.skip_text: names=[x for x in names if x!="text"]
    if args.skip_tables: names=[x for x in names if x!="tables"]
    if args.skip_fields: names=[x for x in names if x!="fields"]
    if args.skip_images: names=[x for x in names if x!="images"]
    return tuple(dict.fromkeys(names))
def task_applies(entry,task,args):
    if task=="tables": return bool(entry["label"].get("tables"))
    if task=="fields": return bool(entry["label"].get("fields"))
    if task=="images": return args.image_scope=="all" or int(entry["label"].get("images") or 0)>0
    return True
def task_complete(record,entry,task,groups,args):
    if not task_applies(entry,task,args): return True
    names=[t.name for t in groups.get(task,[])]
    if not names: return True
    bucket=(record or {}).get(task) or {}
    return all(name in bucket for name in names)
def record_complete(record,entry,tasks,groups,args):
    return all(task_complete(record,entry,task,groups,args) for task in tasks)
def result_resource_error(value):
    if isinstance(value,dict):
        if any(m in str(value.get("error") or "") or m in str(value.get("stderr") or "") or m in str(value.get("stdout") or "") for m in RESOURCE_ERROR_MARKERS): return True
        return any(result_resource_error(v) for v in value.values())
    if isinstance(value,list): return any(result_resource_error(v) for v in value)
    return any(m in str(value) for m in RESOURCE_ERROR_MARKERS)
def run_entry(entry,groups,args,workroot,tasks,prior=None):
    rec=merge_record(base(entry),prior) if prior else base(entry)
    work=workroot/sanitize(entry["id"]); work.mkdir(parents=True,exist_ok=True)
    if "text" in tasks and not task_complete(rec,entry,"text",groups,args):
        rec.setdefault("text",{})
        for t in groups["text"]:
            if t.name not in rec["text"]: rec["text"][t.name]=run_text(entry,t,args,work)
    if "tables" in tasks and not task_complete(rec,entry,"tables",groups,args):
        rec.setdefault("tables",{})
        for t in groups["tables"]:
            if t.name not in rec["tables"]: rec["tables"][t.name]=run_tables(entry,t,args,work)
    if "fields" in tasks and not task_complete(rec,entry,"fields",groups,args):
        rec.setdefault("fields",{})
        for t in groups["fields"]:
            if t.name not in rec["fields"]: rec["fields"][t.name]=run_fields(entry,t,args,work)
    if "images" in tasks and not task_complete(rec,entry,"images",groups,args):
        rec.setdefault("images",{})
        for t in groups["images"]:
            if t.name not in rec["images"]: rec["images"][t.name]=run_images(entry,t,args,work)
    try: work.rmdir()
    except OSError: pass
    return rec
def load_done(path):
    d={}
    if not path.exists(): return d
    for line in path.read_text(encoding="utf-8",errors="replace").splitlines():
        if not line.strip(): continue
        try:
            r=json.loads(line)
            d[r["id"]]=merge_record(d.get(r["id"]),r)
        except Exception: pass
    return d
def pct(vals,p):
    vals=sorted(vals)
    if not vals: return None
    if len(vals)==1: return vals[0]
    pos=(len(vals)-1)*p; lo=math.floor(pos); hi=math.ceil(pos)
    return vals[lo] if lo==hi else vals[lo]+(vals[hi]-vals[lo])*(pos-lo)
def perf_tool(items):
    total=len(items); at=[x for x in items if x is not None]; ok=[x for x in at if x.get("ok")]; times=[x["duration_ms"]/1000 for x in ok if x.get("duration_ms") is not None]; mem=[x["peak_memory_mb"] for x in ok if x.get("peak_memory_mb") is not None]; fails=Counter((x.get("failure_kind") or "error") for x in at if not x.get("ok"))
    return {"files":total,"attempted":len(at),"passed":len(ok),"pass_rate":round(100*len(ok)/total,3) if total else None,"mean_s":round(statistics.fmean(times),6) if times else None,"p50_s":round(pct(times,.5),6) if times else None,"p95_s":round(pct(times,.95),6) if times else None,"p99_s":round(pct(times,.99),6) if times else None,"peak_memory_mb_p95":round(pct(mem,.95),3) if mem else None,"docs_per_sec":round(len(ok)/sum(times),5) if times and sum(times)>0 else None,"failures":dict(fails)}
def perf(records,task,names): return {n:perf_tool([r.get(task,{}).get(n) for r in records]) for n in names}
def metrics(records,task,names,keys):
    out={}
    for n in names:
        rs=[r.get(task,{}).get(n) for r in records if r.get(task,{}).get(n,{}).get("ok")]; row={"scored":len(rs)}
        for k in keys:
            vs=[x.get(k) for x in rs if isinstance(x.get(k),(int,float))]; row[k]=round(statistics.fmean(vs),5) if vs else None
        out[n]=row
    return out
def percat(records,task,names):
    alltags=sorted({t for r in records for t in r.get("tags",[])})
    return {tag:perf([r for r in records if tag in r.get("tags",[])],task,names) for tag in alltags}
def git_commit(): ok,out=run_cap(["git","rev-parse","HEAD"]); return out.strip() if ok else None
def aggregate(records,availability,args,cinfo):
    tn=[t.name for t in avail(text_tools(),availability,args)]; tab=[t.name for t in avail(table_tools(),availability,args)]; fn=[t.name for t in avail(field_tools(),availability,args)]; im=[t.name for t in avail(image_tools(),availability,args)]
    return {"generated_at":datetime.now(timezone.utc).isoformat(),"repo_commit":git_commit(),"python":sys.version.split()[0],"platform":sys.platform,"corpus":cinfo,"availability":availability,"tool_names":{"text":tn,"tables":tab,"fields":fn,"images":im},"text_perf":perf(records,"text",tn),"text_accuracy":metrics(records,"text",tn,["char_similarity","word_f1","line_recall","spurious_line_ratio","reading_order"]),"text_per_category":percat(records,"text",tn),"table_perf":perf(records,"tables",tab),"table_accuracy":metrics(records,"tables",tab,["table_cell_f1","table_cell_recall","table_cell_precision","table_teds_approx"]),"field_perf":perf(records,"fields",fn),"field_accuracy":metrics(records,"fields",fn,["field_f1","field_recall","field_precision","field_value_f1"]),"image_perf":perf(records,"images",im),"image_accuracy":metrics(records,"images",im,["image_count_accuracy","image_count_error"]),"nobody_passed_text":[r["id"] for r in records if not any(x.get("ok") for x in r.get("text",{}).values())],"args":vars(args)}
def fmt(v,n=3):
    if v is None: return "-"
    if isinstance(v,float): return f"{v:.{n}f}"
    return str(v)
def md(headers,rows):
    out=["| "+" | ".join(headers)+" |","| "+" | ".join(["---"]*len(headers))+" |"]
    for r in rows: out.append("| "+" | ".join(str(x) for x in r)+" |")
    return "\n".join(out)
def rank(d,key,rev=False):
    return sorted(d.items(),key=lambda kv:(1,0) if kv[1].get(key) is None else (0,-kv[1][key] if rev else kv[1][key]))
def matrix():
    if PUBLIC_MATRIX.exists():
        try: data=json.loads(PUBLIC_MATRIX.read_text(encoding="utf-8"))
        except Exception: data={}
    else: data={}
    tools=["wellfriendpdf","pdf_wellfriendpdf","pymupdf","pypdfium2","pymupdf4llm","pdftext","pdfminer.six","pdfplumber","markitdown","pypdf","docling","qpdf","poppler"]
    rows=[]
    existing={c.get("name"):c.get("tools",{}) for c in data.get("capabilities",[])}
    def add(name,vals):
        base={t:"no" for t in tools}; base.update(existing.get(name,{})); base.update(vals); rows.append((name,base))
    add("plain text extraction",{"wellfriendpdf":"yes","pdf_wellfriendpdf":"yes","pymupdf":"yes","pypdfium2":"yes","pymupdf4llm":"yes","pdftext":"yes","pdfminer.six":"yes","pdfplumber":"yes","markitdown":"yes","pypdf":"yes","docling":"yes","poppler":"yes"})
    add("chars/words/lines with geometry",{"wellfriendpdf":"partial","pdf_wellfriendpdf":"yes","pymupdf":"yes","pypdfium2":"partial","pdftext":"yes","pdfminer.six":"partial","pdfplumber":"yes","docling":"yes","poppler":"partial"})
    add("layout/reading-order structure",{"wellfriendpdf":"yes","pdf_wellfriendpdf":"yes","pymupdf4llm":"yes","pdftext":"yes","docling":"yes","pymupdf":"partial","pdfplumber":"partial"})
    add("table extraction",{"wellfriendpdf":"yes","pdf_wellfriendpdf":"yes","pymupdf":"yes","pdfplumber":"yes","pymupdf4llm":"yes","docling":"yes","pdftext":"partial"})
    add("image extraction/counting",{"wellfriendpdf":"yes","pdf_wellfriendpdf":"yes","pymupdf":"yes","pypdfium2":"partial","pdfminer.six":"partial","pdfplumber":"partial","pypdf":"partial","poppler":"yes"})
    add("form field read/fill",{"wellfriendpdf":"partial","pdf_wellfriendpdf":"yes","pymupdf":"yes","pypdf":"partial","qpdf":"partial","poppler":"partial","docling":"partial"})
    add("markdown conversion",{"wellfriendpdf":"partial","pdf_wellfriendpdf":"yes","pymupdf4llm":"yes","markitdown":"yes","docling":"yes"})
    add("region/scoped extraction",{"wellfriendpdf":"yes","pdf_wellfriendpdf":"yes","pymupdf":"yes","pdfplumber":"yes","pypdfium2":"partial","docling":"partial"})
    add("extraction profiles",{"wellfriendpdf":"yes","pdf_wellfriendpdf":"yes","pymupdf4llm":"partial","docling":"partial"})
    add("Python/developer API",{"wellfriendpdf":"yes","pdf_wellfriendpdf":"yes","pymupdf":"yes","pypdfium2":"yes","pymupdf4llm":"yes","pdftext":"yes","pdfminer.six":"yes","pdfplumber":"yes","markitdown":"yes","pypdf":"yes","docling":"yes"})
    add("OCR",{"wellfriendpdf":"partial","pdf_wellfriendpdf":"yes","pymupdf":"partial","pymupdf4llm":"partial","markitdown":"partial","docling":"yes","poppler":"partial"})
    add("repair/linearization/validation",{"wellfriendpdf":"yes","qpdf":"yes","poppler":"partial","pypdf":"partial","pymupdf":"partial"})
    add("digital signatures/PDF-A/PDF-UA",{"wellfriendpdf":"yes","pymupdf":"partial","pypdf":"partial"})
    add("MCP/AI assistant integration",{"wellfriendpdf":"no","pdf_wellfriendpdf":"yes","markitdown":"partial","docling":"partial"})
    lacks=set(data.get("wellfriendpdf_lacks",[])); lacks.update(["Docling-class ML layout/OCR is not built into Wellfriend; this release binary is not OCR-enabled.","MCP server/assistant integration advertised by pdf_wellfriendpdf.","qpdf remains the stronger dedicated structural validator and repair reference."])
    lacks.discard("Region/scoped extraction comparable to pdf_wellfriendpdf within/region, PyMuPDF clip, or pdfplumber crop.")
    lacks.discard("ExtractionProfile-style public presets and lazy Python page properties such as page.text/page.words/page.tables/page.images.")
    lacks.discard("Region/scoped extraction API comparable to pdf_wellfriendpdf page.region()/within() and pdfplumber crop().")
    lacks.discard("Documented extraction-profile presets comparable to pdf_wellfriendpdf ExtractionProfile and PyMuPDF4LLM's high-level extraction modes.")
    lacks.discard("Lazy Python page properties such as page.text/page.words/page.tables/page.images.")
    lacks.discard("Markdown heading detection is present only as heuristic document-model output, not an explicit page.markdown(detect_headings=True)-style API.")
    strengths=set(data.get("wellfriendpdf_differentiators",[])); strengths.update(["Self-host HTTP API with auth, rate limits, resource caps, and async render/image jobs.","Pure-Rust core with CLI, Rust library, Python binding, C ABI, and WASM surfaces.","Single product surface spans parse, tables, fields, images, render, edit, optimize, repair, linearize, redact, encrypt, and signatures.","PDF/A/PDF/UA and digital-signature surfaces exceed most extraction-only tools.","Region extraction, extraction profiles, and markdown heading detection are exposed across Rust, CLI, and Python."])
    sources=data.get("sources",{}); sources.update({"docling":["https://docling-project.github.io/docling/"],"qpdf":["https://qpdf.readthedocs.io/"],"poppler":["https://poppler.freedesktop.org/"]})
    return {"tools":tools,"rows":rows,"lacks":sorted(lacks),"strengths":sorted(strengths),"sources":sources}
def write_report(s,records,path):
    c=s["corpus"]; m=matrix(); L=[]; w=L.append
    w("# Competitive Benchmark: Wellfriend vs Major PDF Tools\n")
    w("## Synthetic Corpus Caveat Up Front\n")
    w(f"This run uses {c['files']} synthetic procedurally generated PDFs with paired JSON ground truth. It measures speed and correctness against known labels, but it is not a wild-PDF robustness benchmark. A high pass rate here does not prove robustness against malformed, scanned, handwritten, camera-captured, or adversarial PDFs.\n")
    w("## Provenance\n"); w(md(["item","value"],[["generated",s["generated_at"]],["commit",s.get("repo_commit")],["python",s.get("python")],["platform",s.get("platform")],["timeout",str(s["args"].get("timeout"))+"s"],["memory cap",str(s["args"].get("max_memory_mb"))+" MB"],["pass definition","subprocess exits 0 before timeout/memory cap and writes the expected output artifact"]])); w("")
    w("## Corpus Breakdown\n"); w(md(["metric","value"],[["files",c["files"]],["pages",c["pages"]],["page range",f"{c['min_pages']} to {c['max_pages']}"],["ground-truth images",c["images"]]])); w(""); w(md(["tag/category","files"],[[k,v] for k,v in sorted(c["tags"].items())])); w("")
    active_tools=set()
    for names in s.get("tool_names",{}).values():
        active_tools.update(names)
    w("## Tools Run vs Skipped\n"); w(md(["tool","run","version","reason/license"],[[t,"yes" if i.get("available") and t in active_tools else ("available, not run" if i.get("available") else "no"),i.get("version") or "-",i.get("reason") or i.get("license") or "-"] for t,i in sorted(s["availability"].items())])); w("")
    if not s["availability"].get("docling",{}).get("available"):
        reason=s["availability"].get("docling",{}).get("reason") or "not available"
        w(f"Docling was not run. It was explicitly attempted, but its benchmark interpreter could not import it: {reason}. No Docling numbers are fabricated.\n")
    w("## Speed And Pass Rate: Text Extraction\n"); w(md(["ranked tool","pass %","mean s","p50 s","p95 s","p99 s","mem p95 MB","docs/sec"],[[t,fmt(r.get("pass_rate")),fmt(r.get("mean_s")),fmt(r.get("p50_s")),fmt(r.get("p95_s")),fmt(r.get("p99_s")),fmt(r.get("peak_memory_mb_p95")),fmt(r.get("docs_per_sec"),4)] for t,r in rank(s["text_perf"],"mean_s")])); w(""); w(f"Files nobody passed for text extraction: {len(s.get('nobody_passed_text') or [])}.\n")
    w("### Per-Category Text Speed/Pass Rate\n")
    for tag,cat in sorted(s["text_per_category"].items()):
        w(f"#### {tag}"); w(md(["tool","attempted","pass %","mean s","p95 s"],[[t,r.get("attempted"),fmt(r.get("pass_rate")),fmt(r.get("mean_s")),fmt(r.get("p95_s"))] for t,r in rank(cat,"mean_s")])); w("")
    w("## Accuracy Against Ground Truth\n")
    w("Text scoring normalizes whitespace, then reports character similarity, token F1, ground-truth line recall, spurious line ratio, and order correctness from matched ground-truth line positions. It penalizes missing lines and extra text.\n")
    w(md(["tool","scored","char sim","word F1","line recall","spurious ratio","order"],[[t,r.get("scored"),fmt(r.get("char_similarity")),fmt(r.get("word_f1")),fmt(r.get("line_recall")),fmt(r.get("spurious_line_ratio")),fmt(r.get("reading_order"))] for t,r in rank(s["text_accuracy"],"word_f1",True)])); w("")
    w("### Table Accuracy\n"); w("Table scoring compares ground-truth headers+cells to structured table outputs. False table detections count against precision.\n"); w(md(["tool","scored","cell F1","recall","precision","TEDS approx"],[[t,r.get("scored"),fmt(r.get("table_cell_f1")),fmt(r.get("table_cell_recall")),fmt(r.get("table_cell_precision")),fmt(r.get("table_teds_approx"))] for t,r in rank(s["table_accuracy"],"table_cell_f1",True)])); w("\nTools not shown lack structured table extraction in this harness or were not installed.\n")
    w("### Field / Key-Value Accuracy\n"); w(md(["tool","scored","strict field F1","recall","precision","value-only F1"],[[t,r.get("scored"),fmt(r.get("field_f1")),fmt(r.get("field_recall")),fmt(r.get("field_precision")),fmt(r.get("field_value_f1"))] for t,r in rank(s["field_accuracy"],"field_f1",True)])); w("\nStrict field F1 requires key and value to match; value-only F1 shows values found under different labels.\n")
    w("### Image Count Accuracy\n"); w(md(["tool","scored","count accuracy","mean abs error"],[[t,r.get("scored"),fmt(r.get("image_count_accuracy")),fmt(r.get("image_count_error"))] for t,r in rank(s["image_accuracy"],"image_count_accuracy",True)])); w("")
    w("## Capability Matrix\n"); w(md(["capability"]+m["tools"],[[name]+[vals.get(t,"no") for t in m["tools"]] for name,vals in m["rows"]])); w("")
    w("### What Wellfriend Lacks\n"); [w("- "+x) for x in m["lacks"]]; w("")
    w("### What Wellfriend Uniquely Has / Where It Is Strong\n"); [w("- "+x) for x in m["strengths"]]; w("")
    w("Capability source notes: "+"; ".join(f"{k}: {', '.join(v)}" for k,v in sorted(m["sources"].items()) if isinstance(v,list))); w("")
    w("## Blunt Verdict\n"); verdict(s,w); w("")
    w("## Prioritized Fix List\n"); fixes(s,w); w("")
    path.parent.mkdir(parents=True,exist_ok=True); path.write_text("\n".join(L)+"\n",encoding="utf-8")
def verdict(s,w):
    op=s["text_perf"].get("wellfriendpdf",{}); oa=s["text_accuracy"].get("wellfriendpdf",{})
    faster=[t for t,r in s["text_perf"].items() if r.get("mean_s") is not None and op.get("mean_s") is not None and r["mean_s"]<op["mean_s"]]
    better_char=[t for t,r in s["text_accuracy"].items() if r.get("char_similarity") is not None and oa.get("char_similarity") is not None and r["char_similarity"]>oa["char_similarity"]]
    better_word=[t for t,r in s["text_accuracy"].items() if r.get("word_f1") is not None and oa.get("word_f1") is not None and r["word_f1"]>oa["word_f1"]]
    w(("Wellfriend is not the fastest text extractor. Faster mean wall-time tools: "+", ".join(faster)+".") if faster else "Wellfriend is fastest by mean text wall time among tools that ran, on this capped synthetic corpus.")
    w(("Wellfriend does not lead text character fidelity. Higher mean char-sim tools: "+", ".join(better_char)+".") if better_char else "Wellfriend leads or ties text char-sim among tools that ran, but this does not prove wild-PDF robustness.")
    w(("Higher mean word-F1 tools: "+", ".join(better_word)+".") if better_word else "Wellfriend leads or ties text word-F1 among tools that ran.")
    tab=s["table_accuracy"].get("wellfriendpdf",{}); fld=s["field_accuracy"].get("wellfriendpdf",{})
    if tab.get("table_cell_f1") is not None: w(f"Wellfriend table cell-F1 is {fmt(tab.get('table_cell_f1'))}; precision is {fmt(tab.get('table_cell_precision'))} and TEDS-approx is {fmt(tab.get('table_teds_approx'))}, so structure quality is reported beside content recall.")
    if fld.get("field_f1") is not None: w(f"Wellfriend strict field-F1 is {fmt(fld.get('field_f1'))}; compare value-only F1 to see label-mapping weakness.")
    if not s["availability"].get("docling",{}).get("available"):
        w("Docling remains the biggest unmeasured structure rival in this run; no Docling numbers are guessed.")
def fixes(s,w):
    items=[]; ox=s["text_accuracy"].get("wellfriendpdf",{}); best=max((r.get("word_f1") or 0 for r in s["text_accuracy"].values()),default=0)
    if ox.get("word_f1") is not None and best>ox["word_f1"]: items.append(("text accuracy",f"Close word-F1 gap: Wellfriend {fmt(ox['word_f1'])} vs best {fmt(best)}."))
    op=s["text_perf"].get("wellfriendpdf",{}); bests=min((r.get("mean_s") for r in s["text_perf"].values() if r.get("mean_s") is not None),default=None)
    if bests is not None and op.get("mean_s") is not None and op["mean_s"]>bests: items.append(("text speed",f"Reduce mean text extraction time: Wellfriend {fmt(op['mean_s'])}s vs best {fmt(bests)}s."))
    tab=s["table_accuracy"].get("wellfriendpdf",{})
    if tab.get("table_cell_f1") is not None and tab["table_cell_f1"]<0.9: items.append(("table extraction",f"Improve precision/recall; current cell-F1 {fmt(tab['table_cell_f1'])}."))
    fld=s["field_accuracy"].get("wellfriendpdf",{})
    if fld.get("field_f1") is not None and fld["field_f1"]<0.9: items.append(("field mapping",f"Improve strict KV mapping; field-F1 {fmt(fld['field_f1'])}, value-only F1 {fmt(fld.get('field_value_f1'))}."))
    img=s["image_accuracy"].get("wellfriendpdf",{})
    if img.get("image_count_accuracy") is not None and img["image_count_accuracy"]<0.98: items.append(("image detection",f"Improve image count parity; count accuracy {fmt(img['image_count_accuracy'])}."))
    items.append(("real-world robustness","Keep robustness claims tied to the dedicated 200-PDF robustness report."))
    if not s["availability"].get("docling",{}).get("available"):
        items.append(("Docling measurement","Run Docling in a compatible environment so it is measured honestly."))
    for i,(name,body) in enumerate(items,1): w(f"{i}. **{name}**: {body}")
def args():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--corpus",default=str(DEFAULT_CORPUS)); p.add_argument("--wellfriendpdf-bin",default=str(default_wellfriendpdf())); p.add_argument("--output-dir",default=str(DEFAULT_OUTPUT)); p.add_argument("--report",default=str(DEFAULT_REPORT))
    p.add_argument("--timeout",type=int,default=60); p.add_argument("--max-memory-mb",type=int,default=2048); p.add_argument("--poll-interval-ms",type=int,default=100)
    p.add_argument("--limit",type=int); p.add_argument("--category"); p.add_argument("--batch-start",type=int,default=0); p.add_argument("--batch-size",type=int)
    p.add_argument("--tools"); p.add_argument("--tasks",help="comma-separated task list: text,tables,fields,images. Default: text,tables,fields")
    p.add_argument("--skip-tables",action="store_true"); p.add_argument("--skip-fields",action="store_true"); p.add_argument("--skip-images",action="store_true"); p.add_argument("--skip-text",action="store_true")
    p.add_argument("--image-scope",choices=["all","with-images"],default="all"); p.add_argument("--resume",action="store_true"); p.add_argument("--aggregate-only",action="store_true")
    p.add_argument("--max-workers",type=int,default=4); p.add_argument("--checkpoint-every",type=int,default=100); p.add_argument("--stop-on-resource-error",action=argparse.BooleanOptionalAction,default=True)
    p.add_argument("--docling-no-ocr",action="store_true",help="Run Docling with PdfPipelineOptions(do_ocr=False); useful for born-digital/synthetic PDF text extraction.")
    p.add_argument("--docling-python",help="Optional Python 3.11/3.12 interpreter used only for Docling detection and subprocesses.")
    return p.parse_args()
def main():
    a=args(); out=Path(a.output_dir); out.mkdir(parents=True,exist_ok=True); records_path=out/"records.jsonl"; work=out/"work"; work.mkdir(parents=True,exist_ok=True)
    all_entries=load_entries(Path(a.corpus),a.limit,a.category); start=max(0,a.batch_start); end=None if a.batch_size is None else start+max(0,a.batch_size); entries=all_entries[start:end]
    tasks=requested_tasks(a); availability=detect_all(a); groups={"text":([] if a.skip_text else avail(text_tools(),availability,a)),"tables":avail(table_tools(),availability,a),"fields":avail(field_tools(),availability,a),"images":avail(image_tools(),availability,a)}
    (out/"metadata.json").write_text(json.dumps({"availability":availability,"args":vars(a),"tasks":tasks,"batch":{"start":start,"size":a.batch_size,"selected_files":len(entries)},"corpus":corpus_summary(all_entries)},indent=2,ensure_ascii=False),encoding="utf-8")
    done=load_done(records_path) if (a.resume or a.aggregate_only) else {}
    if not a.aggregate_only:
        pending=[(idx,e,done.get(e["id"])) for idx,e in enumerate(entries,1) if not record_complete(done.get(e["id"]),e,tasks,groups,a)]
        print(f"tasks={','.join(tasks)} files_selected={len(entries)} pending={len(pending)} max_workers={a.max_workers} resume={a.resume}",flush=True)
        mode="a" if a.resume else "w"
        with records_path.open(mode,encoding="utf-8") as fh:
            completed=0
            def finish(idx,e,r):
                nonlocal completed
                done[e["id"]]=merge_record(done.get(e["id"]),r)
                fh.write(json.dumps(done[e["id"]],ensure_ascii=False)+"\n"); fh.flush()
                completed+=1
                if a.checkpoint_every and completed%a.checkpoint_every==0:
                    try: os.fsync(fh.fileno())
                    except OSError: pass
                if completed==1 or completed%25==0 or completed==len(pending): print(f"[{completed}/{len(pending)}] idx={idx}/{len(entries)} {e['id']}",flush=True)
                if a.stop_on_resource_error and result_resource_error(r):
                    raise RuntimeError("Windows resource exhaustion detected in a child result; checkpoint written, stopping run")
            if a.max_workers<=1:
                for idx,e,prior in pending:
                    finish(idx,e,run_entry(e,groups,a,work,tasks,prior))
            else:
                with concurrent.futures.ThreadPoolExecutor(max_workers=a.max_workers) as ex:
                    futs={ex.submit(run_entry,e,groups,a,work,tasks,prior):(idx,e) for idx,e,prior in pending}
                    try:
                        for fut in concurrent.futures.as_completed(futs):
                            idx,e=futs[fut]; finish(idx,e,fut.result())
                    except Exception:
                        for f in futs: f.cancel()
                        raise
    done=load_done(records_path); ids={e["id"] for e in all_entries}; records=[done[k] for k in sorted(done) if k in ids]; summary=aggregate(records,availability,a,corpus_summary(all_entries)); (out/"summary.json").write_text(json.dumps(summary,indent=2,ensure_ascii=False),encoding="utf-8"); write_report(summary,records,Path(a.report)); print(f"wrote {a.report}"); print(f"raw records: {records_path}"); print(f"merged records: {len(records)}")
if __name__=="__main__": raise SystemExit(main())
