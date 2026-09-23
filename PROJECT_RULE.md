# PROJECT RULE

- https://github.com/ajrcarey/pdfium-render 라이브러리를 사용해서 윈도우 11 전용 드로잉 패드 친화적 필기앱을 만들어주세요.

- https://github.com/jaywoo0830a/elm-magic 라이브러리를 사용해서, https://github.com/jaywoo0830a/elm-magic/tree/dev/examples/windows-reactor 예제에 아주 자세한 사용법이 나와있음.

- 60hz, 120hz, 180hz, 240hz 지원해야합니다.

- OTD.SharedMemoryOutput/ 아래 문서 읽고 공유 메모리로 OTD 통해서 데이터를 얻어오세요.

- 코드와 주석은 100% 영어로 작성해주세요.

- 필기감 최적화된 앱이어야 합니다.

- 어떤 것도 UI 스레드를 막으면 안됩니다. 그 어떤 것도! 아키텍쳐 구조상으로 불가능해야합니다.

- 코어는 절대 소스 먼저가 아닌 tests 폴더 아래에 테스트 중심으로 개발해주세요. 반대로 UI에 대해선 어떤 테스트도 작성하지 마세요.

- serfe, thiserror, anyhow 이 3개의 크레이트는 꼭 설치해주세요.
