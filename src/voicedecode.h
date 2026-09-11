#ifndef VOICEDECODE_H
#define VOICEDECODE_H

/// Selects the decoding helper instead of the app. The environment, not argv:
/// same binary, same jail, no second entry in the desktop file.
#define XMATIC_VOICE_DECODE_ENV "XMATIC_VOICE_DECODE"
#define XMATIC_VOICE_IN_ENV "XMATIC_VOICE_IN"
#define XMATIC_VOICE_OUT_ENV "XMATIC_VOICE_OUT"

/// How the helper ended. The parent reads nothing else from it.
enum VoiceDecodeExit {
    VoiceDecodeOk = 0,
    VoiceDecodeFailed = 1,
    VoiceDecodeNotVoice = 2,
    VoiceDecodeTooLong = 3,
    VoiceDecodeTimedOut = 4,
};

/// Decodes one Ogg/Opus voice message to 16 kHz mono PCM WAV, then exits.
int runVoiceDecode();

#endif // VOICEDECODE_H
