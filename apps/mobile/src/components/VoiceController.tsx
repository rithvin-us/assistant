import React, { useState, useRef } from 'react';
import IconButton from '@mui/material/IconButton';
import MicRoundedIcon from '@mui/icons-material/MicRounded';
import MicOffRoundedIcon from '@mui/icons-material/MicOffRounded';
import StopRoundedIcon from '@mui/icons-material/StopRounded';
import CircularProgress from '@mui/material/CircularProgress';
import Chip from '@mui/material/Chip';
import { VoiceState } from '../api/types';
import { transcribeVoiceAudio } from '../api/voice';

interface VoiceControllerProps {
  onTranscriptReady: (transcript: string) => void;
  disabled?: boolean;
}

export const VoiceController: React.FC<VoiceControllerProps> = ({
  onTranscriptReady,
  disabled = false,
}) => {
  const [voiceState, setVoiceState] = useState<VoiceState>('idle');
  const [liveTranscript, setLiveTranscript] = useState<string>('');
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const audioChunksRef = useRef<Blob[]>([]);

  const startRecording = async () => {
    setErrorMessage(null);
    setLiveTranscript('');
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      audioChunksRef.current = [];
      const mediaRecorder = new MediaRecorder(stream);
      mediaRecorderRef.current = mediaRecorder;

      mediaRecorder.ondataavailable = (event) => {
        if (event.data.size > 0) {
          audioChunksRef.current.push(event.data);
        }
      };

      mediaRecorder.onstop = async () => {
        const audioBlob = new Blob(audioChunksRef.current, { type: 'audio/wav' });
        // Stop stream tracks
        stream.getTracks().forEach((track) => track.stop());

        setVoiceState('transcribing');
        try {
          const res = await transcribeVoiceAudio(audioBlob);
          if (res.text && res.text.trim().length > 0) {
            setLiveTranscript(res.text);
            setVoiceState('thinking');
            // This control's job ends at the transcript. The conversation
            // sends it through the assistant core like any typed turn, and
            // renders the answer.
            //
            // It used to synthesise the transcript itself and play that back,
            // so the user heard their own question read out in the assistant's
            // voice and the assistant's actual answer was never spoken. Speaking
            // a reply is `useVoiceTurn`'s job, and it speaks the real one.
            onTranscriptReady(res.text);
          }
          setVoiceState('idle');
        } catch (err: unknown) {
          setErrorMessage(err instanceof Error ? err.message : 'Transcription failed');
          setVoiceState('error');
          setTimeout(() => setVoiceState('idle'), 3000);
        }
      };

      mediaRecorder.start();
      setVoiceState('listening');
    } catch {
      setErrorMessage('Microphone access denied or unavailable');
      setVoiceState('error');
      setTimeout(() => setVoiceState('idle'), 3000);
    }
  };

  const stopRecording = () => {
    if (mediaRecorderRef.current && mediaRecorderRef.current.state !== 'inactive') {
      mediaRecorderRef.current.stop();
    }
  };

  const handleInterrupt = () => {
    if (mediaRecorderRef.current && mediaRecorderRef.current.state !== 'inactive') {
      mediaRecorderRef.current.stop();
    }
    setVoiceState('interrupted');
    setTimeout(() => setVoiceState('idle'), 1000);
  };

  const getStateBadge = () => {
    switch (voiceState) {
      case 'listening':
        return <Chip label="Listening..." color="primary" size="small" variant="filled" className="animate-pulse" />;
      case 'transcribing':
        return <Chip label="Transcribing..." color="secondary" size="small" variant="outlined" />;
      case 'thinking':
        return <Chip label="Thinking..." color="info" size="small" variant="outlined" />;
      case 'speaking':
        return <Chip label="Speaking..." color="success" size="small" variant="filled" className="animate-pulse" />;
      case 'interrupted':
        return <Chip label="Interrupted" color="warning" size="small" variant="outlined" />;
      case 'error':
        return <Chip label={errorMessage || 'Error'} color="error" size="small" variant="filled" />;
      default:
        return null;
    }
  };

  return (
    <div className="flex items-center gap-2">
      {getStateBadge()}

      {liveTranscript && (voiceState === 'transcribing' || voiceState === 'thinking' || voiceState === 'speaking') && (
        <span className="text-xs italic text-gray-400 max-w-[150px] truncate">{liveTranscript}</span>
      )}

      {voiceState === 'listening' ? (
        <IconButton
          onClick={stopRecording}
          color="error"
          size="medium"
          title="Stop Recording"
          className="bg-rose-500/20 hover:bg-rose-500/30 text-rose-500 p-2 rounded-full border border-rose-500/40"
        >
          <StopRoundedIcon />
        </IconButton>
      ) : voiceState === 'speaking' ? (
        <IconButton
          onClick={handleInterrupt}
          color="warning"
          size="medium"
          title="Interrupt / Stop"
          className="bg-amber-500/20 hover:bg-amber-500/30 text-amber-500 p-2 rounded-full border border-amber-500/40"
        >
          <MicOffRoundedIcon />
        </IconButton>
      ) : (
        <IconButton
          onClick={startRecording}
          disabled={disabled || (voiceState as VoiceState) !== 'idle'}
          color="primary"
          size="medium"
          title="Tap to Speak"
          className="bg-sky-500/10 hover:bg-sky-500/20 text-sky-400 p-2 rounded-full border border-sky-500/30 disabled:opacity-40"
        >
          {voiceState === 'transcribing' || voiceState === 'thinking' ? (
            <CircularProgress size={20} color="inherit" />
          ) : (
            <MicRoundedIcon />
          )}
        </IconButton>
      )}
    </div>
  );
};
