params ["_vehicle"];

_vehicle animateSource ["Winchsection", 0];
_vehicle animateSource ["Winchsection2", 0];

_vehicle animateDoor ["dvere_p", 1];

_vehicle setVariable ["ace_fastroping_doorsLocked", false, true];

2
