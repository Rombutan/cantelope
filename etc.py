import can
from time import sleep
import time

import sys
import select

def get_int_with_default(default_value):
    """
    Checks if there is data waiting in stdin. 
    If yes, reads it and tries to return an int.
    If no data is available immediately, returns the default.
    """
    
    # Check if stdin has data (Unix-based logic)
    # On Windows, select only works on sockets, so we use a different check
    if sys.platform != 'win32':
        # select.select([files], [outputs], [exceptions], timeout)
        i, _, _ = select.select([sys.stdin], [], [], 0.1)
        if i:
            line = sys.stdin.readline().strip()
            try:
                return int(line)
            except ValueError:
                return default_value
    else:
        # Windows-specific: This is trickier for pipe/redirected input.
        # For a simple 'is data there' check:
        import msvcrt
        if msvcrt.kbhit():
            line = sys.stdin.readline().strip()
            try:
                return int(line)
            except ValueError:
                return default_value

    return default_value

def isData():
    return select.select([sys.stdin], [], [], 0) == ([sys.stdin], [], [])

bus = can.Bus(interface='socketcan', channel='can0', bitrate=500000)
start = time.time()
i = 0
throttle = 0
while(True):
    msg = can.Message(
        arbitration_id=646,
        data=[100,00,100,00,00,00,00,00],
        is_extended_id=False
    )
    try:
        bus.send(msg)

    except can.CanError:
        print("Message NOT sent")


    throttle = get_int_with_default(throttle)
    if(throttle < 0.1):
        prB = 1
    else:
        prB = 9

    msg = can.Message(
        arbitration_id=390,
        data=[00,00,0x4c,0x1d,0x8,i,00,00],
        is_extended_id=False
    )
    
    try:
        bus.send(msg)

    except can.CanError:
        print("Message NOT sent")

    
    msg = can.Message(
        arbitration_id=80,
        data = 0,
        is_extended_id = False
        )
    try:
        bus.send(msg);
    except can.CanError:
        print("Sync not sent")


    i = i + 1
    if(i >= 16):
        i = 0

    sleep(0.02)

