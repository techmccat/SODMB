I wrote this code to reduce my used memory:
every compressed audio used to get stored in memory when added to the queue.

Turns out that keeping the pipe with youtube-dl and ffmpeg open
uses a lot more RAM than just storing the whole song, so there's that.
