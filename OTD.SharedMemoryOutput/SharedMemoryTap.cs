// OTD 공유 메모리 탭 — 태블릿 리포트를 **변환 전** 그대로 공유 메모리에 내보낸다.
//
// ## 왜 출력 모드(IOutputMode)가 아니라 필터인가
// 출력 모드는 프로필당 **하나뿐**이다. 그것을 우리 것으로 바꾸면 지금 쓰는
// `VoiDPlugins.OutputMode.WinInkAbsoluteMode`(커서·Windows Ink 출력)를 우리가 다시 구현해야 한다.
// 반면 `PreTransform` 필터는 파이프라인에 **한 칸 끼어들 뿐**이고, 리포트를 그대로 통과시키면
// 기존 출력 모드가 아무 영향 없이 계속 동작한다. 게다가 PreTransform의 좌표는 **태블릿 단위**라
// 창 크기·화면 배율에 흔들리지 않는다(페이지 매핑은 앱이 한다).
//
// ## 지켜야 하는 것 (하나라도 어기면 펜이 죽는다)
// 1. `Consume`은 디바이스 리더 스레드에서 **동기 호출**된다 → 절대 기다리지 않는다. 쓰기만 한다.
// 2. `Emit?.Invoke(report)`를 반드시 부른다 → 안 부르면 파이프라인이 끊겨 커서가 멈춘다.
// 3. 표식(`slot.seq`) 순서를 지킨다 → 반쪽 샘플을 읽는 쪽에 노출하지 않는다.
//
// 계약 전문: 같은 폴더의 `PROTOCOL.md`

using System;
using System.Diagnostics;
using System.IO.MemoryMappedFiles;
using System.Runtime.CompilerServices;
using System.Runtime.Versioning;
using System.Threading;
using OpenTabletDriver.Plugin;
using OpenTabletDriver.Plugin.Attributes;
using OpenTabletDriver.Plugin.Output;
using OpenTabletDriver.Plugin.Tablet;

// OTD 자체가 Windows 전용이고 공유 메모리도 Windows API다 — 플랫폼 분석기에 그렇게 알린다.
[assembly: SupportedOSPlatform("windows")]

namespace OTD.SharedMemoryOutput
{
    [PluginName("Shared Memory Tap (light-note)")]
    public sealed unsafe class SharedMemoryTap : IPositionedPipelineElement<IDeviceReport>
    {
        // ── 계약 상수 (PROTOCOL.md와 반드시 같아야 한다) ──────────────────────
        public const string MapName = "light-note.otd.shm";
        private const ulong Magic = 0x4C4E_4F54_4453_4D31; // "LNOTDSM1"
        private const uint Version = 1;
        private const int SampleSize = 64;
        private const int Capacity = 4096;
        private const int HeaderSize = 128;
        private const int TotalSize = HeaderSize + Capacity * SampleSize;

        private const int OffWriteSeq = 0x18;
        private const int OffHeartbeat = 0x38;
        private const int OffSlotSeq = 0x00;
        private const int OffX = 0x08;
        private const int OffY = 0x0C;
        private const int OffPressure = 0x10;
        private const int OffTiltX = 0x14;
        private const int OffTiltY = 0x18;
        private const int OffRotation = 0x1C;
        private const int OffFlags = 0x20;
        private const int OffHover = 0x24;
        private const int OffTime = 0x28;

        private const uint FlagTip = 1 << 0;
        private const uint FlagEraser = 1 << 1;
        private const uint FlagOutOfRange = 1 << 2;
        private const uint FlagProximity = 1 << 3;
        private const int ButtonShift = 4;

        /// <summary>프로세스당 하나. 필터가 여러 번 생성되어도 매핑은 하나다.</summary>
        private static readonly object Gate = new();
        private static Shared? shared;

        /// <summary>PreTransform = 태블릿 단위(변환 전). 이 위치라야 기존 출력 모드가 그대로 동작한다.</summary>
        public PipelinePosition Position => PipelinePosition.PreTransform;

        /// `IPipelineElement<T>.Emit`은 널 허용 주석이 꺼진 어셈블리라 `Action<IDeviceReport>`다
        /// (소스의 `T?`는 널 허용이 꺼져 있으면 그냥 `T`다) — 서명을 그대로 맞춘다.
        public event Action<IDeviceReport>? Emit;

        public void Consume(IDeviceReport report)
        {
            if (report is null)
            {
                // 앞 단계가 null을 흘렸을 수 있다(영역 제한 필터가 그렇게 한다). 우리가 삼키면
                // 파이프라인이 끊긴다 — 그대로 넘긴다. 이때는 기록할 것이 없다.
                Emit?.Invoke(null!);
                return;
            }

            Publish(report);

            // 2번 규칙: 통과시키지 않으면 펜 입력이 여기서 끝난다.
            Emit?.Invoke(report);
        }

        private static void Publish(IDeviceReport report)
        {
            var tap = shared ??= Create();
            if (tap is null)
                return;

            var flags = 0u;
            var x = 0f;
            var y = 0f;
            var pressure = 0f;
            var tiltX = 0f;
            var tiltY = 0f;

            // 리포트 종류는 **인터페이스로** 판단한다(파서마다 클래스가 다르다).
            if (report is ITabletReport tablet)
            {
                x = tablet.Position.X;
                y = tablet.Position.Y;
                pressure = tablet.Pressure;
                if (pressure > 0)
                    flags |= FlagTip;

                var buttons = tablet.PenButtons;
                if (buttons is not null)
                {
                    for (var i = 0; i < buttons.Length && i < 8; i++)
                        if (buttons[i])
                            flags |= 1u << (ButtonShift + i);
                }
            }
            else if (report is IAbsolutePositionReport absolute)
            {
                x = absolute.Position.X;
                y = absolute.Position.Y;
            }

            if (report is ITiltReport tilt)
            {
                tiltX = tilt.Tilt.X;
                tiltY = tilt.Tilt.Y;
            }

            if (report is IEraserReport eraser && eraser.Eraser)
                flags |= FlagEraser;

            if (report is IProximityReport)
                flags |= FlagProximity;

            if (report is OutOfRangeReport)
            {
                flags = FlagOutOfRange; // 좌표는 의미가 없다 — 진행 중인 획을 끝내는 신호다.
                x = y = pressure = 0f;
            }

            // 회전은 0.6.7에 리포트 인터페이스가 없다(`IRotationReport`는 이후 브랜치에만 있다)
            // — 계약에는 자리를 남겨 두고 0을 쓴다.
            tap.Write(flags, x, y, pressure, tiltX, tiltY, 0f);
        }

        private static Shared? Create()
        {
            lock (Gate)
            {
                if (shared is not null)
                    return shared;

                try
                {
                    shared = new Shared();
                }
                catch (Exception ex)
                {
                    // 매핑 실패는 치명적이지 않다 — 필터는 통과만 해도 무해하다.
                    Log.Write(nameof(SharedMemoryTap), $"공유 메모리를 만들지 못했습니다: {ex.Message}");
                }

                return shared;
            }
        }

        /// <summary>매핑 하나와 그 위의 쓰기 경로. **할당 없음, 시스템 호출 없음.**</summary>
        private sealed unsafe class Shared
        {
            private readonly MemoryMappedFile file;
            private readonly MemoryMappedViewAccessor view;
            private readonly byte* origin;

            public Shared()
            {
                file = MemoryMappedFile.CreateOrOpen(MapName, TotalSize, MemoryMappedFileAccess.ReadWrite);
                view = file.CreateViewAccessor(0, TotalSize, MemoryMappedFileAccess.ReadWrite);

                byte* pointer = null;
                view.SafeMemoryMappedViewHandle.AcquirePointer(ref pointer);
                origin = pointer;

                // 헤더 — 낡은 매핑을 물려받았을 수 있으므로 전부 다시 쓴다.
                Unsafe.WriteUnaligned(origin + 0x00, Magic);
                Unsafe.WriteUnaligned(origin + 0x08, Version);
                Unsafe.WriteUnaligned(origin + 0x0C, (uint)SampleSize);
                Unsafe.WriteUnaligned(origin + 0x10, (uint)Capacity);
                Unsafe.WriteUnaligned(origin + 0x14, 1u); // bit0 = 살아 있음
                Unsafe.WriteUnaligned(origin + OffWriteSeq, 0ul);
                Unsafe.WriteUnaligned(origin + 0x20, 0ul);
                // 태블릿 범위는 필터가 알 수 없다(출력 모드만 `Tablet`을 안다) → 앱이 RPC로 채운다.
                Unsafe.WriteUnaligned(origin + 0x28, 0f);
                Unsafe.WriteUnaligned(origin + 0x2C, 0f);
                Unsafe.WriteUnaligned(origin + 0x30, 0f);
                Unsafe.WriteUnaligned(origin + 0x34, 0u);
                Unsafe.WriteUnaligned(origin + OffHeartbeat, 0ul);
                new Span<byte>(origin + 0x40, 64).Clear();
            }

            public void Write(uint flags, float x, float y, float pressure, float tiltX, float tiltY, float rotation)
            {
                // 3번 규칙: 번호를 원자적으로 받고 → "쓰는 중" → 필드 → "완성".
                var seq = (ulong)Interlocked.Increment(ref Unsafe.AsRef<long>(origin + OffWriteSeq));
                var slot = origin + HeaderSize + (seq % Capacity) * SampleSize;

                Unsafe.WriteUnaligned(slot + OffSlotSeq, seq * 2 + 1);
                Unsafe.WriteUnaligned(slot + OffX, x);
                Unsafe.WriteUnaligned(slot + OffY, y);
                Unsafe.WriteUnaligned(slot + OffPressure, pressure);
                Unsafe.WriteUnaligned(slot + OffTiltX, tiltX);
                Unsafe.WriteUnaligned(slot + OffTiltY, tiltY);
                Unsafe.WriteUnaligned(slot + OffRotation, rotation);
                Unsafe.WriteUnaligned(slot + OffFlags, flags);
                Unsafe.WriteUnaligned(slot + OffHover, 0u); // v1 미사용
                Unsafe.WriteUnaligned(slot + OffTime, (ulong)Stopwatch.GetTimestamp());
                Unsafe.WriteUnaligned(slot + OffSlotSeq, seq * 2);

                // 심장 박동 — 앱이 "플러그인이 살아 있는가"를 판단하는 근거다.
                Unsafe.WriteUnaligned(origin + OffHeartbeat, (ulong)Stopwatch.GetTimestamp());
            }
        }
    }
}
