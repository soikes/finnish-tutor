import React, { useState, useRef, useCallback, useEffect } from "react";
import {
  StyleSheet,
  Text,
  View,
  TouchableOpacity,
  ScrollView,
} from "react-native";
import { StatusBar } from "expo-status-bar";
import { Audio } from "expo-av";
import LiveAudioStream from "react-native-live-audio-stream";

// --- Configuration ---
// Server listens on WireGuard interface — no TLS needed, tunnel is encrypted
const SERVER_URL = "ws://10.0.0.1:8080/ws";

type AppStatus = "idle" | "connecting" | "listening" | "processing" | "speaking";

interface Message {
  role: "user" | "assistant";
  text: string;
}

export default function App() {
  const [status, setStatus] = useState<AppStatus>("idle");
  const [messages, setMessages] = useState<Message[]>([]);
  const [micActive, setMicActive] = useState(false);

  const wsRef = useRef<WebSocket | null>(null);
  const audioQueueRef = useRef<ArrayBuffer[]>([]);
  const isPlayingRef = useRef(false);
  const scrollViewRef = useRef<ScrollView>(null);
  const streamInitRef = useRef(false);
  const micActiveRef = useRef(false);

  // Connect WebSocket
  const connect = useCallback(() => {
    console.log("[WS] connect() called, current state:", wsRef.current?.readyState);
    if (wsRef.current?.readyState === WebSocket.OPEN) return;

    setStatus("connecting");
    console.log("[WS] Connecting to", SERVER_URL);
    const ws = new WebSocket(SERVER_URL);

    ws.binaryType = "arraybuffer";

    ws.onopen = () => {
      console.log("[WS] Connected");
      setStatus("idle");
      ws.send(JSON.stringify({ type: "start" }));
    };

    ws.onmessage = (event) => {
      if (typeof event.data === "string") {
        const msg = JSON.parse(event.data);
        switch (msg.type) {
          case "transcription":
            setMessages((prev) => [
              ...prev,
              { role: "user", text: msg.text },
            ]);
            setStatus("processing");
            break;
          case "response":
            setMessages((prev) => [
              ...prev,
              { role: "assistant", text: msg.text },
            ]);
            break;
          case "audio_start":
            setStatus("speaking");
            break;
          case "audio_end":
            playAudioQueue().then(() => {
              setStatus(micActive ? "listening" : "idle");
            });
            break;
          case "error":
            console.error("Server error:", msg.message);
            setStatus("idle");
            break;
        }
      } else {
        // Binary audio data from TTS
        audioQueueRef.current.push(event.data as ArrayBuffer);
      }
    };

    ws.onerror = (error) => {
      console.error("[WS] Error:", error);
      setStatus("idle");
    };

    ws.onclose = (event) => {
      console.log("[WS] Closed, code:", event.code, "reason:", event.reason);
      setStatus("idle");
      wsRef.current = null;
    };

    wsRef.current = ws;
  }, []);

  // Play received audio
  const playAudioQueue = async () => {
    if (isPlayingRef.current || audioQueueRef.current.length === 0) return;
    isPlayingRef.current = true;

    try {
      // Concatenate all audio buffers
      const totalLength = audioQueueRef.current.reduce(
        (sum, buf) => sum + buf.byteLength,
        0
      );
      const combined = new Uint8Array(totalLength);
      let offset = 0;
      for (const buf of audioQueueRef.current) {
        combined.set(new Uint8Array(buf), offset);
        offset += buf.byteLength;
      }
      audioQueueRef.current = [];

      // Create WAV header for the PCM data
      // Server sends 16-bit LE PCM at 22050 Hz (Piper TTS output)
      const wavBuffer = createWavBuffer(combined, 22050);

      // Convert to base64 for expo-av
      const base64 = arrayBufferToBase64(wavBuffer);
      const uri = `data:audio/wav;base64,${base64}`;

      // Pause recording so iOS releases the audio session for playback
      LiveAudioStream.stop();

      await Audio.setAudioModeAsync({
        allowsRecordingIOS: false,
        playsInSilentModeIOS: true,
      });

      const { sound } = await Audio.Sound.createAsync({ uri });
      await sound.playAsync();

      // Wait for playback to finish
      await new Promise<void>((resolve) => {
        sound.setOnPlaybackStatusUpdate((playbackStatus) => {
          if (playbackStatus.isLoaded && playbackStatus.didJustFinish) {
            resolve();
          }
        });
      });

      await sound.unloadAsync();

      // Resume recording if mic is still active
      if (micActiveRef.current) {
        await Audio.setAudioModeAsync({
          allowsRecordingIOS: true,
          playsInSilentModeIOS: true,
        });
        LiveAudioStream.start();
      }
    } catch (error) {
      console.error("Audio playback error:", error);
    } finally {
      isPlayingRef.current = false;
    }
  };

  // Initialize live audio stream (once)
  const initStream = () => {
    if (streamInitRef.current) return;
    streamInitRef.current = true;

    LiveAudioStream.init({
      sampleRate: 16000,
      channels: 1,
      bitsPerSample: 16,
      audioSource: 6, // VOICE_RECOGNITION (Android)
      bufferSize: 4096, // ~256ms at 16kHz
    } as any);

    LiveAudioStream.on("data", (base64: string) => {
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        // Decode base64 to binary and send as PCM
        const binary = atob(base64);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) {
          bytes[i] = binary.charCodeAt(i);
        }
        console.log("[AUDIO] Sending", bytes.length, "bytes");
        wsRef.current.send(bytes.buffer);
      }
    });
  };

  // Start recording
  const startRecording = async () => {
    console.log("[MIC] startRecording called");
    try {
      const permission = await Audio.requestPermissionsAsync();
      console.log("[MIC] Permission:", permission.granted);
      if (!permission.granted) {
        console.error("[MIC] Microphone permission not granted");
        return;
      }

      await Audio.setAudioModeAsync({
        allowsRecordingIOS: true,
        playsInSilentModeIOS: true,
      });

      // Connect if not already
      connect();

      initStream();
      LiveAudioStream.start();
      console.log("[MIC] LiveAudioStream started");

      setMicActive(true);
      micActiveRef.current = true;
      setStatus("listening");
    } catch (error) {
      console.error("Failed to start recording:", error);
    }
  };

  // Stop recording
  const stopRecording = async () => {
    console.log("[MIC] stopRecording called");
    LiveAudioStream.stop();
    setMicActive(false);
    micActiveRef.current = false;
    setStatus("idle");
  };

  // Toggle mic
  const toggleMic = () => {
    console.log("[MIC] toggleMic, micActive:", micActive);
    if (micActive) {
      stopRecording();
    } else {
      startRecording();
    }
  };

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    scrollViewRef.current?.scrollToEnd({ animated: true });
  }, [messages]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      LiveAudioStream.stop();
      wsRef.current?.close();
    };
  }, []);

  const statusText: Record<AppStatus, string> = {
    idle: "Tap the mic to start",
    connecting: "Connecting...",
    listening: "Listening...",
    processing: "Thinking...",
    speaking: "Speaking...",
  };

  const statusColor: Record<AppStatus, string> = {
    idle: "#666",
    connecting: "#f0ad4e",
    listening: "#4ecdc4",
    processing: "#f0ad4e",
    speaking: "#6c5ce7",
  };

  return (
    <View style={styles.container}>
      <StatusBar style="light" />

      <Text style={styles.title}>Finnish Tutor</Text>
      <Text style={[styles.status, { color: statusColor[status] }]}>
        {statusText[status]}
      </Text>

      <ScrollView
        ref={scrollViewRef}
        style={styles.messageList}
        contentContainerStyle={styles.messageContent}
      >
        {messages.map((msg, i) => (
          <View
            key={i}
            style={[
              styles.messageBubble,
              msg.role === "user" ? styles.userBubble : styles.assistantBubble,
            ]}
          >
            <Text
              style={[
                styles.messageText,
                msg.role === "user"
                  ? styles.userText
                  : styles.assistantText,
              ]}
            >
              {renderFinnishTags(msg.text, msg.role === "user" ? "#1a1a2e" : "#4ecdc4")}
            </Text>
          </View>
        ))}
      </ScrollView>

      <TouchableOpacity
        style={[
          styles.micButton,
          micActive && styles.micButtonActive,
        ]}
        onPress={toggleMic}
        activeOpacity={0.7}
      >
        <Text style={styles.micIcon}>{micActive ? "⏹" : "🎤"}</Text>
      </TouchableOpacity>
    </View>
  );
}

// --- Text rendering ---

function renderFinnishTags(text: string, boldColor: string): React.ReactNode[] {
  const parts: React.ReactNode[] = [];
  let remaining = text;
  let key = 0;

  while (remaining.length > 0) {
    const start = remaining.indexOf("[fi]");
    if (start === -1) {
      parts.push(remaining);
      break;
    }

    if (start > 0) {
      parts.push(remaining.slice(0, start));
    }

    const afterTag = remaining.slice(start + 4);
    const end = afterTag.indexOf("[/fi]");
    if (end === -1) {
      parts.push(
        <Text key={key++} style={{ fontWeight: "bold", color: boldColor }}>
          {afterTag}
        </Text>
      );
      break;
    }

    parts.push(
      <Text key={key++} style={{ fontWeight: "bold", color: boldColor }}>
        {afterTag.slice(0, end)}
      </Text>
    );
    remaining = afterTag.slice(end + 5);
  }

  return parts;
}

// --- Utility functions ---

function createWavBuffer(pcmData: Uint8Array, sampleRate: number): ArrayBuffer {
  const numChannels = 1;
  const bitsPerSample = 16;
  const byteRate = (sampleRate * numChannels * bitsPerSample) / 8;
  const blockAlign = (numChannels * bitsPerSample) / 8;
  const dataSize = pcmData.length;
  const buffer = new ArrayBuffer(44 + dataSize);
  const view = new DataView(buffer);

  // RIFF header
  writeString(view, 0, "RIFF");
  view.setUint32(4, 36 + dataSize, true);
  writeString(view, 8, "WAVE");

  // fmt chunk
  writeString(view, 12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, numChannels, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, byteRate, true);
  view.setUint16(32, blockAlign, true);
  view.setUint16(34, bitsPerSample, true);

  // data chunk
  writeString(view, 36, "data");
  view.setUint32(40, dataSize, true);
  new Uint8Array(buffer, 44).set(pcmData);

  return buffer;
}

function writeString(view: DataView, offset: number, str: string) {
  for (let i = 0; i < str.length; i++) {
    view.setUint8(offset + i, str.charCodeAt(i));
  }
}

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

// --- Styles ---

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#1a1a2e",
    paddingTop: 60,
    paddingHorizontal: 20,
  },
  title: {
    fontSize: 28,
    fontWeight: "bold",
    color: "#fff",
    textAlign: "center",
    marginBottom: 4,
  },
  status: {
    fontSize: 14,
    textAlign: "center",
    marginBottom: 16,
  },
  messageList: {
    flex: 1,
    marginBottom: 20,
  },
  messageContent: {
    paddingBottom: 10,
  },
  messageBubble: {
    maxWidth: "85%",
    padding: 12,
    borderRadius: 16,
    marginBottom: 8,
  },
  userBubble: {
    alignSelf: "flex-end",
    backgroundColor: "#4ecdc4",
    borderBottomRightRadius: 4,
  },
  assistantBubble: {
    alignSelf: "flex-start",
    backgroundColor: "#2d2d44",
    borderBottomLeftRadius: 4,
  },
  messageText: {
    fontSize: 16,
    lineHeight: 22,
  },
  userText: {
    color: "#1a1a2e",
  },
  assistantText: {
    color: "#e0e0e0",
  },
  micButton: {
    width: 80,
    height: 80,
    borderRadius: 40,
    backgroundColor: "#2d2d44",
    alignSelf: "center",
    justifyContent: "center",
    alignItems: "center",
    marginBottom: 40,
    borderWidth: 3,
    borderColor: "#4ecdc4",
  },
  micButtonActive: {
    backgroundColor: "#4ecdc4",
    borderColor: "#ff6b6b",
  },
  micIcon: {
    fontSize: 32,
  },
});
