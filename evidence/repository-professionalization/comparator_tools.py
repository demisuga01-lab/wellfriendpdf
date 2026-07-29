import importlib.util, importlib
mods=['pypdfium2','fitz','pikepdf','pdfplumber','camelot']
for m in mods:
    if importlib.util.find_spec(m) is None:
        print(m, 'unavailable')
    else:
        mod=importlib.import_module(m)
        print(m, getattr(mod, '__version__', 'available'))
