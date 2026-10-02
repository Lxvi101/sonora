#import <Foundation/Foundation.h>
#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <CoreMedia/CoreMedia.h>

typedef int (*CancelCheck)(void *);
static void fail(char *out, size_t cap, NSString *message) {
    if (out && cap) snprintf(out, cap, "%s", message.UTF8String ?: "Could not extract audio.");
}
// Runs on a worker. Only the first audio track is read; no video frames are decoded.
int sonora_extract_audio(const char *source, const char *directory, CancelCheck cancelled,
                         void *context, int *kind, char *error, size_t cap) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:source isDirectory:NO relativeToURL:nil];
        NSURL *dir = [NSURL fileURLWithFileSystemRepresentation:directory isDirectory:YES relativeToURL:nil];
        AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:nil];
        AVAssetTrack *track = [asset tracksWithMediaType:AVMediaTypeAudio].firstObject;
        if (cancelled(context)) { fail(error, cap, @"Import cancelled."); return -1; }
        if (!track) { fail(error, cap, @"This file has no readable audio track. Try an MP4, MOV or M4V with audio."); return -1; }
        NSError *err = nil;
        AVMutableComposition *composition = [AVMutableComposition composition];
        AVMutableCompositionTrack *audio = [composition addMutableTrackWithMediaType:AVMediaTypeAudio preferredTrackID:kCMPersistentTrackID_Invalid];
        if (![audio insertTimeRange:track.timeRange ofTrack:track atTime:kCMTimeZero error:&err]) {
            fail(error, cap, err.localizedDescription); return -1;
        }
        // AAC/ALAC commonly remux without quality loss or a full PCM intermediate.
        AVAssetExportSession *session = [[AVAssetExportSession alloc] initWithAsset:composition presetName:AVAssetExportPresetPassthrough];
        NSURL *compressed = [dir URLByAppendingPathComponent:@"audio.m4a"];
        if ([session.supportedFileTypes containsObject:AVFileTypeAppleM4A]) {
            session.outputURL = compressed; session.outputFileType = AVFileTypeAppleM4A;
            dispatch_semaphore_t done = dispatch_semaphore_create(0);
            [session exportAsynchronouslyWithCompletionHandler:^{ dispatch_semaphore_signal(done); }];
            while (dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 50 * NSEC_PER_MSEC))) {
                if (cancelled(context)) [session cancelExport];
            }
            if (session.status == AVAssetExportSessionStatusCompleted && !cancelled(context)) { *kind = 0; return 0; }
            [[NSFileManager defaultManager] removeItemAtURL:compressed error:nil];
        }
        if (cancelled(context)) { fail(error, cap, @"Import cancelled."); return -1; }
        // Lossless streaming fallback for PCM and other decodable movie soundtracks.
        AVAssetReader *reader = [[AVAssetReader alloc] initWithAsset:composition error:&err];
        NSDictionary *settings = @{ AVFormatIDKey: @(kAudioFormatLinearPCM), AVLinearPCMBitDepthKey: @32,
            AVLinearPCMIsFloatKey: @YES, AVLinearPCMIsBigEndianKey: @NO, AVLinearPCMIsNonInterleaved: @NO };
        AVAssetReaderTrackOutput *output = [[AVAssetReaderTrackOutput alloc] initWithTrack:audio outputSettings:settings];
        output.alwaysCopiesSampleData = NO;
        if (!reader || ![reader canAddOutput:output]) { fail(error, cap, err.localizedDescription ?: @"This video audio codec is unsupported."); return -1; }
        [reader addOutput:output];
        if (![reader startReading]) { fail(error, cap, reader.error.localizedDescription); return -1; }
        ExtAudioFileRef file = NULL;
        NSURL *pcm = [dir URLByAppendingPathComponent:@"audio.caf"];
        OSStatus status = noErr;
        uint64_t total = 0;
        while (reader.status == AVAssetReaderStatusReading && !cancelled(context)) {
            @autoreleasepool {
                CMSampleBufferRef sample = [output copyNextSampleBuffer];
                if (!sample) break;
                const AudioStreamBasicDescription *format = CMAudioFormatDescriptionGetStreamBasicDescription(CMSampleBufferGetFormatDescription(sample));
                if (!format || format->mChannelsPerFrame == 0 || format->mChannelsPerFrame > 32) { CFRelease(sample); status = -50; break; }
                if (!file) {
                    status = ExtAudioFileCreateWithURL((__bridge CFURLRef)pcm, kAudioFileCAFType, format, NULL, kAudioFileFlags_EraseFile, &file);
                    if (!status) status = ExtAudioFileSetProperty(file, kExtAudioFileProperty_ClientDataFormat, sizeof(*format), format);
                }
                AudioBufferList list = {0}; CMBlockBufferRef block = NULL;
                if (!status) status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(sample, NULL, &list, sizeof(list), NULL, NULL, kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment, &block);
                if (!status && list.mNumberBuffers != 1) status = -50;
                if (!status) status = ExtAudioFileWrite(file, (UInt32)CMSampleBufferGetNumSamples(sample), &list);
                total += CMSampleBufferGetNumSamples(sample);
                if (block) CFRelease(block);
                CFRelease(sample);
                if (status) break;
            }
        }
        BOOL wasCancelled = cancelled(context);
        BOOL complete = reader.status == AVAssetReaderStatusCompleted;
        if (!complete) [reader cancelReading];
        if (file) { OSStatus close = ExtAudioFileDispose(file); if (!status) status = close; }
        if (wasCancelled || status || !complete || !total) {
            [[NSFileManager defaultManager] removeItemAtURL:pcm error:nil];
            fail(error, cap, wasCancelled ? @"Import cancelled." : (reader.error.localizedDescription ?: @"Could not decode the video's audio track."));
            return -1;
        }
        *kind = 1; return 0;
    }
}
