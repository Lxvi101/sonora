#import <Foundation/Foundation.h>
#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <AppKit/AppKit.h>
#include <stdint.h>
#include <stdio.h>

static void error_text(char *out, size_t cap, NSString *text) {
    if (out && cap) snprintf(out, cap, "%s", text.UTF8String ?: "Unknown audio error");
}

typedef struct { ExtAudioFileRef file; AudioStreamBasicDescription format; int64_t frames; } Reader;
void *sonora_reader_open(const char *path, double *rate, uint32_t *channels, uint64_t *frames, char *err, size_t cap) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        Reader *r = calloc(1, sizeof(Reader));
        OSStatus status = ExtAudioFileOpenURL((__bridge CFURLRef)url, &r->file);
        if (!status) {
            UInt32 n = sizeof(r->format);
            status = ExtAudioFileGetProperty(r->file, kExtAudioFileProperty_FileDataFormat, &n, &r->format);
        }
        if (!status) { UInt32 n = sizeof(r->frames); status = ExtAudioFileGetProperty(r->file, kExtAudioFileProperty_FileLengthFrames, &n, &r->frames); }
        if (!status && (r->format.mChannelsPerFrame == 0 || r->format.mChannelsPerFrame > 32 || r->format.mSampleRate <= 0 || r->frames <= 0)) status = -50;
        if (!status) {
            AudioStreamBasicDescription client = {0};
            client.mSampleRate = r->format.mSampleRate;
            client.mFormatID = kAudioFormatLinearPCM;
            client.mFormatFlags = kAudioFormatFlagsNativeFloatPacked;
            client.mBitsPerChannel = 32;
            client.mChannelsPerFrame = r->format.mChannelsPerFrame;
            client.mFramesPerPacket = 1;
            client.mBytesPerFrame = client.mBytesPerPacket = 4 * client.mChannelsPerFrame;
            status = ExtAudioFileSetProperty(r->file, kExtAudioFileProperty_ClientDataFormat, sizeof(client), &client);
        }
        if (status) {
            error_text(err, cap, [NSString stringWithFormat:@"Cannot decode this audio file (Core Audio %d). Try WAV, AIFF, MP3, M4A, CAF, or FLAC.", (int)status]);
            if (r->file) ExtAudioFileDispose(r->file);
            free(r); return NULL;
        }
        *rate = r->format.mSampleRate; *channels = r->format.mChannelsPerFrame; *frames = (uint64_t)r->frames;
        return r;
    }
}
int sonora_reader_seek(void *ptr, uint64_t frame) { return ExtAudioFileSeek(((Reader *)ptr)->file, (SInt64)frame); }
int sonora_reader_read(void *ptr, float *samples, uint32_t capacity, uint32_t *read) {
    Reader *r = ptr;
    AudioBufferList buffers = { .mNumberBuffers = 1, .mBuffers = {{ .mNumberChannels = r->format.mChannelsPerFrame, .mDataByteSize = capacity * r->format.mChannelsPerFrame * 4, .mData = samples }} };
    *read = capacity;
    return ExtAudioFileRead(r->file, read, &buffers);
}
void sonora_reader_close(void *ptr) { Reader *r = ptr; if (r) { ExtAudioFileDispose(r->file); free(r); } }

@interface SonoraPlayer : NSObject
@property AVAudioEngine *engine;
@property AVAudioPlayerNode *node;
@property AVAudioUnitEQ *gain;
@property AVAudioFile *file;
@property double start;
@property double end;
@property double paused;
@property BOOL running;
// Arranged playback. `generation` (atomic; written only on the main thread)
// invalidates earlier schedules. Everything below it is touched only on
// `queue`, the one place segments are scheduled from.
@property uint64_t generation;
@property dispatch_queue_t queue;
@property AVAudioFile *scheduledFile;
@property NSData *ranges;
@property NSUInteger next;
@property uint64_t nextFrame;
@property NSUInteger queued;
@property uint64_t queuedFrames;
@end
@implementation SonoraPlayer
@end

// Bounded look-ahead: a few seconds or segments, never the whole arrangement.
static const NSUInteger SONORA_MAX_QUEUED = 256;
static const double SONORA_AHEAD_SECONDS = 2.0;

// Runs on p.queue. Each consumed segment schedules more, so joins are queued
// back to back on the node long before they play.
static void sonora_schedule_more(SonoraPlayer *p, uint64_t generation) {
    // Uses the file captured with this schedule, never `p.file`, which the
    // main thread may replace once the schedule has been forgotten.
    AVAudioFile *file = p.scheduledFile;
    NSData *ranges = p.ranges;
    if (p.generation != generation || !file || !ranges) return;
    const uint64_t *r = ranges.bytes;
    NSUInteger count = ranges.length / (2 * sizeof(uint64_t));
    uint64_t ahead = (uint64_t)(file.processingFormat.sampleRate * SONORA_AHEAD_SECONDS);
    __weak SonoraPlayer *weak = p;
    // Re-checked per segment: a forgotten schedule stops adding at once.
    while (p.generation == generation && p.next < count && p.queued < SONORA_MAX_QUEUED &&
           (p.queued < 2 || p.queuedFrames < ahead)) {
        uint64_t end = r[2 * p.next + 1];
        uint64_t frame = MAX(p.nextFrame, r[2 * p.next]);
        AVAudioFrameCount chunk = (AVAudioFrameCount)MIN(end - frame, (uint64_t)UINT32_MAX);
        p.queued += 1; p.queuedFrames += chunk;
        p.nextFrame = frame + chunk;
        if (p.nextFrame >= end) { p.next += 1; p.nextFrame = 0; }
        @try {
            [p.node scheduleSegment:file startingFrame:(AVAudioFramePosition)frame frameCount:chunk atTime:nil
                completionCallbackType:AVAudioPlayerNodeCompletionDataConsumed
                     completionHandler:^(__unused AVAudioPlayerNodeCompletionCallbackType type) {
                SonoraPlayer *owner = weak;
                if (!owner || owner.generation != generation) return;
                dispatch_async(owner.queue, ^{
                    if (owner.generation != generation) return;
                    owner.queued -= 1; owner.queuedFrames -= chunk;
                    sonora_schedule_more(owner, generation);
                });
            }];
        } @catch (NSException *e) {
            p.next = count; return;
        }
    }
}
// Invalidates the arranged schedule, then waits out any scheduling pass
// already running on `queue`, so once this returns no stale segment can reach
// the node and nothing on `queue` reads the old file or ranges. Call it before
// stopping the node, rescheduling or replacing the file.
//
// Main thread only, never from `queue` (dispatch_sync onto itself would
// deadlock). Safe from deadlock otherwise: `queue` blocks only schedule on
// the node and never wait on the main thread, and node completion handlers
// merely dispatch_async onto `queue`; later blocks see the new generation
// and return without touching anything.
static void sonora_forget_schedule(SonoraPlayer *p) {
    p.generation += 1;
    dispatch_sync(p.queue, ^{
        p.scheduledFile = nil; p.ranges = nil;
        p.next = 0; p.nextFrame = 0; p.queued = 0; p.queuedFrames = 0;
    });
}

void *sonora_player_new(void) {
    @autoreleasepool {
        SonoraPlayer *p = [SonoraPlayer new];
        p.engine = [AVAudioEngine new]; p.node = [AVAudioPlayerNode new]; p.gain = [[AVAudioUnitEQ alloc] initWithNumberOfBands:0];
        p.queue = dispatch_queue_create("io.sonora.player.schedule", DISPATCH_QUEUE_SERIAL);
        [p.engine attachNode:p.node]; [p.engine attachNode:p.gain];
        return (__bridge_retained void *)p;
    }
}
int sonora_player_load(void *ptr, const char *path, char *err, size_t cap) {
    @autoreleasepool {
        SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
        sonora_forget_schedule(p);
        [p.node stop]; [p.engine stop]; p.running = NO; p.paused = 0;
        NSError *error = nil;
        p.file = [[AVAudioFile alloc] initForReading:[NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil] error:&error];
        if (!p.file) { error_text(err, cap, error.localizedDescription); return -1; }
        [p.engine disconnectNodeOutput:p.node]; [p.engine disconnectNodeOutput:p.gain];
        @try {
            [p.engine connect:p.node to:p.gain format:p.file.processingFormat];
            [p.engine connect:p.gain to:p.engine.mainMixerNode format:p.file.processingFormat];
        } @catch (NSException *e) { error_text(err, cap, e.reason); return -1; }
        return 0;
    }
}
int sonora_player_play(void *ptr, double start, double end, float db, char *err, size_t cap) {
    @autoreleasepool {
        SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
        if (!p.file) { error_text(err, cap, @"Open an audio file first."); return -1; }
        double rate = p.file.processingFormat.sampleRate;
        int64_t first = llround(start * rate), last = MIN(p.file.length, llround(end * rate));
        if (first < 0 || last <= first || last - first > UINT32_MAX) { error_text(err, cap, @"Choose a valid playback range shorter than 24 hours."); return -1; }
        sonora_forget_schedule(p);
        [p.node stop]; p.running = NO;
        p.start = (double)first / rate; p.end = (double)last / rate; p.paused = p.start; p.gain.globalGain = db;
        NSError *error = nil;
        @try {
            [p.node scheduleSegment:p.file startingFrame:first frameCount:(AVAudioFrameCount)(last-first) atTime:nil completionHandler:nil];
            if (!p.engine.isRunning && ![p.engine startAndReturnError:&error]) { error_text(err, cap, error.localizedDescription); return -1; }
            [p.node play]; p.running = YES;
        } @catch (NSException *e) { error_text(err, cap, e.reason); return -1; }
        return 0;
    }
}
// Play source segments back to back on one player node. Joins are queued on
// the node ahead of time (never by the UI timer), with a bounded look-ahead;
// only the range list (16 bytes per clip) is retained, never PCM.
int sonora_player_play_ranges(void *ptr, const uint64_t *starts, const uint64_t *ends,
                              size_t count, double timeline_start, float db,
                              char *err, size_t cap) {
    @autoreleasepool {
        SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
        if (!p.file || !count || !isfinite(timeline_start) || timeline_start < 0) {
            error_text(err, cap, @"Choose a non-empty playback range."); return -1;
        }
        double rate = p.file.processingFormat.sampleRate;
        uint64_t total = 0;
        NSMutableData *ranges = [NSMutableData dataWithLength:count * 2 * sizeof(uint64_t)];
        uint64_t *r = ranges.mutableBytes;
        for (size_t i = 0; i < count; i++) {
            if (ends[i] <= starts[i] || ends[i] > (uint64_t)p.file.length ||
                UINT64_MAX - total < ends[i] - starts[i]) {
                error_text(err, cap, @"Invalid source clip."); return -1;
            }
            total += ends[i] - starts[i];
            r[2 * i] = starts[i]; r[2 * i + 1] = ends[i];
        }
        sonora_forget_schedule(p);
        [p.node stop]; p.running = NO;
        p.start = timeline_start; p.end = timeline_start + (double)total / rate;
        p.paused = p.start; p.gain.globalGain = db;
        uint64_t generation = p.generation;
        AVAudioFile *file = p.file;
        dispatch_sync(p.queue, ^{
            p.scheduledFile = file; p.ranges = ranges;
            p.next = 0; p.nextFrame = 0; p.queued = 0; p.queuedFrames = 0;
            sonora_schedule_more(p, generation);
        });
        NSError *error = nil;
        @try {
            if (!p.engine.isRunning && ![p.engine startAndReturnError:&error]) {
                sonora_forget_schedule(p);
                [p.node stop]; error_text(err, cap, error.localizedDescription); return -1;
            }
            [p.node play]; p.running = YES;
        } @catch (NSException *e) {
            sonora_forget_schedule(p);
            [p.node stop]; error_text(err, cap, e.reason); return -1;
        }
        return 0;
    }
}

double sonora_player_position(void *ptr) {
    SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
    if (!p.running) return p.paused;
    AVAudioTime *render = p.node.lastRenderTime;
    AVAudioTime *time = render ? [p.node playerTimeForNodeTime:render] : nil;
    if (!time || !time.isSampleTimeValid || time.sampleRate <= 0) return p.start;
    return MIN(p.end, p.start + MAX(0, (double)time.sampleTime / time.sampleRate));
}
int sonora_player_is_playing(void *ptr) {
    SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
    if (!p.running) return 0;
    if (sonora_player_position(ptr) >= p.end) {
        sonora_forget_schedule(p);
        p.paused = p.end; p.running = NO; [p.node stop]; [p.engine pause]; return 0;
    }
    return 1;
}
void sonora_player_pause(void *ptr) {
    SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
    p.paused = sonora_player_position(ptr); p.running = NO; [p.node pause]; [p.engine pause];
}
void sonora_player_stop(void *ptr) {
    SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
    sonora_forget_schedule(p);
    p.running = NO; p.paused = 0; [p.node stop]; [p.engine pause];
}
void sonora_player_gain(void *ptr, float db) { ((__bridge SonoraPlayer *)ptr).gain.globalGain = db; }
void sonora_player_free(void *ptr) {
    @autoreleasepool { SonoraPlayer *p = CFBridgingRelease(ptr); sonora_forget_schedule(p); [p.node stop]; [p.engine stop]; }
}
void sonora_reveal(const char *path) {
    @autoreleasepool { [[NSWorkspace sharedWorkspace] activateFileViewerSelectingURLs:@[[NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil]]]; }
}
static void fix_glass(NSView *view) {
    if ([view isKindOfClass:NSVisualEffectView.class]) {
        NSVisualEffectView *glass = (NSVisualEffectView *)view;
        glass.material = NSVisualEffectMaterialUnderWindowBackground;
        glass.blendingMode = NSVisualEffectBlendingModeBehindWindow;
        glass.state = NSVisualEffectStateActive;
    }
    for (NSView *child in view.subviews) fix_glass(child);
}
void sonora_apply_vibrancy(void) {
    for (NSWindow *window in NSApp.windows) {
        window.appearance = [NSAppearance appearanceNamed:NSAppearanceNameDarkAqua];
        fix_glass(window.contentView.superview ?: window.contentView);
    }
}

// Streaming compressed export. These writers never retain an entire recording.
#include <lame/lame.h>
#include <unistd.h>
typedef struct {
    int kind;
    uint32_t channels;
    ExtAudioFileRef aac;
    lame_t mp3;
    FILE *file;
    unsigned char encoded[65536];
} SonoraWriter;

void *sonora_writer_open(const char *path, int kind, double rate, uint32_t channels, uint64_t frames, char *err, size_t cap) {
    @autoreleasepool {
        if (channels < 1 || channels > 2) { error_text(err, cap, @"MP3 and M4A support mono or stereo. Use WAV for multichannel audio."); return NULL; }
        SonoraWriter *w = calloc(1, sizeof(*w)); w->kind = kind; w->channels = channels;
        if (kind == 1) {
            w->mp3 = lame_init();
            int output_rate = rate >= 48000 ? 48000 : (rate >= 44100 ? 44100 : 32000);
            if (!w->mp3 || lame_set_in_samplerate(w->mp3, (int)llround(rate)) < 0 ||
                lame_set_out_samplerate(w->mp3, output_rate) < 0 ||
                lame_set_num_channels(w->mp3, channels) < 0 ||
                lame_set_num_samples(w->mp3, (unsigned long)frames) < 0 ||
                lame_set_brate(w->mp3, 192) < 0 || lame_set_quality(w->mp3, 2) < 0 ||
                lame_set_bWriteVbrTag(w->mp3, 1) < 0 || lame_init_params(w->mp3) < 0) {
                error_text(err, cap, @"Could not initialize MP3 encoding for this recording.");
                if (w->mp3) lame_close(w->mp3); free(w); return NULL;
            }
            // Rust creates this file exclusively before calling us.
            w->file = fopen(path, "r+b");
            if (!w->file) { error_text(err, cap, @"Could not open the temporary export file."); lame_close(w->mp3); free(w); return NULL; }
        } else if (kind == 2) {
            AudioStreamBasicDescription output = {0};
            // 192 kbps AAC is supported for mono and stereo at these rates.
            output.mSampleRate = rate >= 48000 ? 48000 : 44100;
            output.mFormatID = kAudioFormatMPEG4AAC;
            output.mChannelsPerFrame = channels;
            UInt32 size = sizeof(output);
            OSStatus status = AudioFormatGetProperty(kAudioFormatProperty_FormatInfo, 0, NULL, &size, &output);
            NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
            if (!status) status = ExtAudioFileCreateWithURL((__bridge CFURLRef)url, kAudioFileM4AType, &output, NULL, kAudioFileFlags_EraseFile, &w->aac);
            AudioStreamBasicDescription client = { .mSampleRate = rate, .mFormatID = kAudioFormatLinearPCM, .mFormatFlags = kAudioFormatFlagsNativeFloatPacked, .mBytesPerPacket = channels * 4, .mFramesPerPacket = 1, .mBytesPerFrame = channels * 4, .mChannelsPerFrame = channels, .mBitsPerChannel = 32 };
            if (!status) status = ExtAudioFileSetProperty(w->aac, kExtAudioFileProperty_ClientDataFormat, sizeof(client), &client);
            AudioConverterRef converter = NULL;
            size = sizeof(converter);
            if (!status) status = ExtAudioFileGetProperty(w->aac, kExtAudioFileProperty_AudioConverter, &size, &converter);
            UInt32 bitrate = 192000;
            if (!status) status = AudioConverterSetProperty(converter, kAudioConverterEncodeBitRate, sizeof(bitrate), &bitrate);
            CFArrayRef config = NULL;
            if (!status) status = ExtAudioFileSetProperty(w->aac, kExtAudioFileProperty_ConverterConfig, sizeof(config), &config);
            if (status) {
                error_text(err, cap, [NSString stringWithFormat:@"M4A encoder unavailable (audio error %d).", (int)status]);
                if (w->aac) ExtAudioFileDispose(w->aac); free(w); return NULL;
            }
        } else { free(w); error_text(err, cap, @"Unknown output format."); return NULL; }
        return w;
    }
}
int sonora_writer_write(void *ptr, const float *samples, uint32_t frames) {
    SonoraWriter *w = ptr;
    if (w->kind == 1) {
        int bytes = w->channels == 1
            ? lame_encode_buffer_ieee_float(w->mp3, samples, samples, (int)frames, w->encoded, sizeof(w->encoded))
            : lame_encode_buffer_interleaved_ieee_float(w->mp3, samples, (int)frames, w->encoded, sizeof(w->encoded));
        if (bytes < 0) return bytes;
        return fwrite(w->encoded, 1, bytes, w->file) == (size_t)bytes ? 0 : -1;
    }
    AudioBufferList buffers = { .mNumberBuffers = 1, .mBuffers = {{ .mNumberChannels = w->channels, .mDataByteSize = frames * w->channels * 4, .mData = (void *)samples }} };
    return ExtAudioFileWrite(w->aac, frames, &buffers);
}
int sonora_writer_close(void *ptr, int finish) {
    SonoraWriter *w = ptr; int status = 0;
    if (w->kind == 1) {
        if (finish) {
            int bytes = lame_encode_flush(w->mp3, w->encoded, sizeof(w->encoded));
            if (bytes < 0 || fwrite(w->encoded, 1, bytes, w->file) != (size_t)bytes) status = -1;
            if (!status) {
                size_t tag = lame_get_lametag_frame(w->mp3, w->encoded, sizeof(w->encoded));
                if (tag > sizeof(w->encoded) || fseek(w->file, 0, SEEK_SET) != 0 || fwrite(w->encoded, 1, tag, w->file) != tag) status = -1;
            }
            if (fflush(w->file) != 0 || fsync(fileno(w->file)) != 0) status = -1;
        }
        if (fclose(w->file) != 0) status = -1;
        lame_close(w->mp3);
    } else {
        status = ExtAudioFileDispose(w->aac);
    }
    free(w); return status;
}
