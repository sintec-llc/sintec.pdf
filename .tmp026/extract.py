import sys,re
data=open(sys.argv[1],'rb').read()
out=sys.argv[2]; k=0
for m in re.finditer(rb'/DCTDecode',data):
    s=data.find(b'stream',m.end()); st=data.find(b'\xff\xd8',s,s+20)
    e=data.find(b'endstream',st)
    k+=1
    if k in (1,5,30):
        open(out+'%d.jpg'%k,'wb').write(data[st:e].rstrip(b'\r\n'))
