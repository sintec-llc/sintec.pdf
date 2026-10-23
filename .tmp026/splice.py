import sys
p = sys.argv[1]
part = open(sys.argv[2], encoding='utf-8').read()
s = open(p, encoding='utf-8').read()
start = s.index("pub(crate) fn resample(")
end = s.index("/// One image: resample and/or recompress per `settings`")
s = s[:start] + part + s[end:]
open(p, 'w', encoding='utf-8').write(s)
