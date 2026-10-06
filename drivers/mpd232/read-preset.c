#include <alsa/asoundlib.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>

/* Read an MPD232 editor preset without changing it.
 * Arguments: Remote ALSA raw-MIDI port, preset 1-30, unused output path.
 * Returns zero after validating and saving a complete preset, or nonzero on failure.
 */
int main(int argc, char **argv) {
    if (argc != 4) { fprintf(stderr, "Usage: %s hw:card,0,3 preset-number output.syx\n", argv[0]); return 2; }
    char *end;
    long preset = strtol(argv[2], &end, 10);
    if (*end || preset < 1 || preset > 30) return 2;
    snd_rawmidi_t *in = NULL, *out = NULL;
    int error = snd_rawmidi_open(&in, &out, argv[1], SND_RAWMIDI_NONBLOCK);
    if (error < 0) { fprintf(stderr, "Remote port: %s\n", snd_strerror(error)); return 1; }
    unsigned char query[] = {0xf0, 0x47, 0, 0x36, 0x12, 0, 1, (unsigned char)(preset - 1), 0xf7};
    snd_rawmidi_drop(in);
    if (snd_rawmidi_write(out, query, sizeof(query)) != sizeof(query)) { fprintf(stderr, "Preset request failed\n"); error = 1; goto close; }
    unsigned char bytes[3484];
    size_t length = 0;
    struct timespec start, now;
    clock_gettime(CLOCK_MONOTONIC, &start);
    for (;;) {
        ssize_t count = snd_rawmidi_read(in, bytes + length, sizeof(bytes) - length);
        if (count > 0) length += (size_t)count;
        else if (count < 0 && count != -EAGAIN) { error = 1; goto close; }
        if (length && bytes[length - 1] == 0xf7) break;
        if (length == sizeof(bytes)) { error = 1; goto close; }
        clock_gettime(CLOCK_MONOTONIC, &now);
        if (now.tv_sec - start.tv_sec >= 3) { fprintf(stderr, "Preset read timed out (%zu bytes)\n", length); error = 1; goto close; }
        usleep(500);
    }
    if (length != 3483 || bytes[0] != 0xf0 || bytes[1] != 0x47 || bytes[2] != 0 || bytes[3] != 0x36 || bytes[4] != 0x10 || bytes[5] != 0x1b || bytes[6] != 0x13 || bytes[7] != preset - 1) { fprintf(stderr, "Invalid preset response\n"); error = 1; goto close; }
    for (size_t index = 7; index + 1 < length; index++) { if (bytes[index] > 127) { error = 1; goto close; } }
    int file = open(argv[3], O_WRONLY | O_CREAT | O_EXCL, 0644);
    if (file < 0) { perror("Preset output"); error = 1; goto close; }
    if (write(file, bytes, length) != (ssize_t)length || fsync(file) < 0) { perror("Preset save"); close(file); unlink(argv[3]); error = 1; goto close; }
    close(file);
    fprintf(stdout, "Read preset %ld: %.8s (%zu bytes); hardware unchanged\n", preset, bytes + 8, length);
    error = 0;
close:
    snd_rawmidi_close(in);
    snd_rawmidi_close(out);
    return error;
}
