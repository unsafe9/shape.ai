데이터 저장소를 인터페이스화 (sqlite, postgres, file, s3, remote server 등등)
모든 어댑터는 export와 import 구현
export는 파일이나 sqlite 등 정해진 하나의 규격으로 파일로 내보내기, 병렬화 등 intensive 하게 내보낼수있는포맷
import는 정해진 export 규격에서 현재 내 구현으로 가져오기
그 외에 인메모리 파일규격에 맞춰서 필요한 모든 입출력 (저장로드) 함수 구현

