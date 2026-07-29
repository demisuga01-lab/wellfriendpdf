import json, os, shutil, statistics, subprocess, sys, tempfile, time
from pathlib import Path

src = Path('/home/demisuga01/wellpdf/tmp/repository-professionalization-20260729T192340Z/src')
out_dir = src / 'benchmarks' / 'results' / 'latest'
out_dir.mkdir(parents=True, exist_ok=True)
input_pdf = src / 'crates' / 'engine' / 'tests' / 'fixtures' / 'multi_stream.pdf'
image_pdf = src / 'crates' / 'engine' / 'tests' / 'fixtures' / 'image_only.pdf'

def version(cmd):
    exe=shutil.which(cmd[0])
    if not exe:
        return {'available': False}
    try:
        p=subprocess.run(cmd, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=10)
        return {'available': True, 'version': p.stdout.splitlines()[0] if p.stdout.splitlines() else exe}
    except Exception as e:
        return {'available': True, 'version': type(e).__name__}

def run_task(name, tool, operation, func, iterations=10):
    times=[]; failures=[]; last={}
    for _ in range(2):
        try: func()
        except Exception: pass
    for _ in range(iterations):
        t=time.perf_counter()
        try:
            last=func() or {}
            times.append((time.perf_counter()-t)*1000.0)
        except Exception as e:
            failures.append(type(e).__name__ + ': ' + str(e)[:160])
    times.sort()
    median=times[len(times)//2] if times else 0.0
    p95=times[min(len(times)-1, round((len(times)-1)*0.95))] if times else 0.0
    return {'task':name,'tool':tool,'operation':operation,'iterations':iterations,'successes':len(times),'failures':len(failures),'median_ms':median,'p95_ms':p95,'last_result':last,'failure_samples':failures[:2],'evidence_class':'measured_directly' if times else 'unavailable'}

results=[]
if shutil.which('qpdf'):
    results.append(run_task('qpdf_check','qpdf','structural_check',lambda: {'exit': subprocess.run(['qpdf','--check',str(input_pdf)], stdout=subprocess.PIPE, stderr=subprocess.PIPE).returncode}))
    def qpdf_rewrite():
        with tempfile.TemporaryDirectory() as d:
            out=Path(d)/'out.pdf'
            p=subprocess.run(['qpdf',str(input_pdf),str(out)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            if p.returncode: raise RuntimeError(p.stderr.decode('utf-8','replace')[:120])
            return {'output_bytes': out.stat().st_size}
    results.append(run_task('qpdf_rewrite','qpdf','structural_rewrite',qpdf_rewrite))
if shutil.which('pdfinfo'):
    results.append(run_task('poppler_pdfinfo','Poppler pdfinfo','page_count',lambda: {'exit': subprocess.run(['pdfinfo',str(input_pdf)], stdout=subprocess.PIPE, stderr=subprocess.PIPE).returncode}))
if shutil.which('pdftotext'):
    def pdftotext():
        p=subprocess.run(['pdftotext',str(input_pdf),'-'], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if p.returncode: raise RuntimeError(p.stderr.decode('utf-8','replace')[:120])
        return {'text_bytes': len(p.stdout)}
    results.append(run_task('poppler_pdftotext','Poppler pdftotext','text_extraction',pdftotext))
try:
    import pikepdf
    def pike_open_save():
        with tempfile.TemporaryDirectory() as d:
            out=Path(d)/'out.pdf'
            with pikepdf.open(input_pdf) as pdf:
                pages=len(pdf.pages)
                pdf.save(out)
            return {'pages':pages,'output_bytes':out.stat().st_size}
    results.append(run_task('pikepdf_open_save','pikepdf (qpdf wrapper)','open_save',pike_open_save))
except Exception as e:
    results.append({'task':'pikepdf_open_save','tool':'pikepdf (qpdf wrapper)','operation':'open_save','successes':0,'failures':1,'evidence_class':'unavailable','failure_samples':[str(e)[:160]]})
try:
    import pypdfium2 as pdfium
    def pdfium_count():
        pdf=pdfium.PdfDocument(str(input_pdf))
        n=len(pdf)
        pdf.close()
        return {'pages': n}
    results.append(run_task('pypdfium2_page_count','pypdfium2 (PDFium wrapper)','page_count',pdfium_count))
except Exception as e:
    results.append({'task':'pypdfium2_page_count','tool':'pypdfium2 (PDFium wrapper)','operation':'page_count','successes':0,'failures':1,'evidence_class':'unavailable','failure_samples':[str(e)[:160]]})
try:
    import fitz
    def pymupdf_text_render():
        doc=fitz.open(str(input_pdf))
        page=doc[0]
        text=page.get_text()
        pix=page.get_pixmap(matrix=fitz.Matrix(1,1), alpha=False)
        return {'pages': doc.page_count, 'text_chars': len(text), 'pixels': pix.width*pix.height}
    results.append(run_task('pymupdf_text_render','PyMuPDF (MuPDF wrapper)','text_and_render',pymupdf_text_render))
except Exception as e:
    results.append({'task':'pymupdf_text_render','tool':'PyMuPDF (MuPDF wrapper)','operation':'text_and_render','successes':0,'failures':1,'evidence_class':'unavailable','failure_samples':[str(e)[:160]]})
try:
    import pdfplumber
    def plumber_text():
        with pdfplumber.open(str(input_pdf)) as pdf:
            text='\n'.join((p.extract_text() or '') for p in pdf.pages)
        return {'text_chars':len(text)}
    results.append(run_task('pdfplumber_text','pdfplumber','text_extraction',plumber_text))
except Exception as e:
    results.append({'task':'pdfplumber_text','tool':'pdfplumber','operation':'text_extraction','successes':0,'failures':1,'evidence_class':'unavailable','failure_samples':[str(e)[:160]]})

tools={'qpdf':version(['qpdf','--version']),'pdfinfo':version(['pdfinfo','-v']),'pdftotext':version(['pdftotext','-v']),'mutool':version(['mutool','-v'])}
report={'schema_version':'repository_professionalization.comparator_benchmark.v1','input':str(input_pdf.relative_to(src)),'image_input':str(image_pdf.relative_to(src)),'results':results,'tools':tools,'notes':['Wrappers are reported with their underlying engine relationship.','Unavailable tools are not scored as failures.']}
(out_dir/'comparator-results.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
print(json.dumps({'result_count':len(results),'output':str(out_dir/'comparator-results.json')}))
