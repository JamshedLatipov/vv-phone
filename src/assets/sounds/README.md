# Softphone Sound Files

This directory contains audio files for the softphone component.

## Current Files

- **ring.mp3** - Incoming call ringtone (plays in loop)
- **ring-calling.mp3** - Outgoing call ringback tone (plays while dialing)
- **busy.mp3** - Busy signal / error tone (short beep)
- **hangup.mp3** - Call ended / hangup tone (short tone when call disconnects)

## Creating hangup.mp3

The `hangup.mp3` file should be a short (0.5-1 second), subtle tone that plays when:
- A call ends normally (remote party hangs up)
- You hang up a call
- Call disconnects normally

### Recommended characteristics:
- **Duration**: 500-800ms
- **Type**: Single beep or two-tone sequence
- **Frequency**: 400-600 Hz (lower/softer than busy tone)
- **Volume**: Moderate (will be played at 40% volume in code)

### Creating the file:

#### Option 1: Use Audacity (Free)
1. Download Audacity from https://www.audacityteam.org/
2. Generate > Tone
3. Settings:
   - Waveform: Sine
   - Frequency: 440 Hz
   - Amplitude: 0.5
   - Duration: 0.5 seconds
4. Effect > Fade Out (last 0.2 seconds)
5. Export as MP3

#### Option 2: Use online tone generator
1. Go to https://www.szynalski.com/tone-generator/
2. Set frequency to 440 Hz
3. Record 0.5 seconds
4. Save as MP3

#### Option 3: Use ffmpeg
```bash
ffmpeg -f lavfi -i "sine=frequency=440:duration=0.5" -af "afade=t=out:st=0.3:d=0.2" hangup.mp3
```

#### Option 4: Duplicate busy.mp3 as temporary solution
```bash
# From this directory
cp busy.mp3 hangup.mp3
```

## Fallback Behavior

If `hangup.mp3` is not found, the application will show a warning in the console but continue to function normally without the hangup sound.
