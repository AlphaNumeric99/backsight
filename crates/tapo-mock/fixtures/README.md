# tapo-mock fixtures

Generated with ffmpeg (test patterns only, no camera footage):

```bash
ffmpeg -f lavfi -i "testsrc2=size=640x360:rate=15" -t 10 -c:v libx264 -profile:v main \
  -pix_fmt yuv420p -bf 0 -g 30 -b:v 600k -f mpegts video.mpegts
ffmpeg -f lavfi -i "testsrc2=size=640x360:rate=1" -frames:v 1 -q:v 5 thumbnail.jpg
```
