#include "wintun.h"

WINTUN_CREATE_ADAPTER_FUNC WintunCreateAdapter;
WINTUN_CLOSE_ADAPTER_FUNC WintunCloseAdapter;
WINTUN_OPEN_ADAPTER_FUNC WintunOpenAdapter;
WINTUN_GET_ADAPTER_LUID_FUNC WintunGetAdapterLUID;
WINTUN_GET_RUNNING_DRIVER_VERSION_FUNC WintunGetRunningDriverVersion;
WINTUN_DELETE_DRIVER_FUNC WintunDeleteDriver;
WINTUN_SET_LOGGER_FUNC WintunSetLogger;
WINTUN_START_SESSION_FUNC WintunStartSession;
WINTUN_END_SESSION_FUNC WintunEndSession;
WINTUN_GET_READ_WAIT_EVENT_FUNC WintunGetReadWaitEvent;
WINTUN_RECEIVE_PACKET_FUNC WintunReceivePacket;
WINTUN_RELEASE_RECEIVE_PACKET_FUNC WintunReleaseReceivePacket;
WINTUN_ALLOCATE_SEND_PACKET_FUNC WintunAllocateSendPacket;
WINTUN_SEND_PACKET_FUNC WintunSendPacket;