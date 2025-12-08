params ["_vehicle"];

_vehicle animateSource ["Winchsection", 1];
_vehicle animateSource ["Winchsection2", 1];

_vehicle animateDoor ["dvere_p", 0];

_vehicle setVariable ["ace_fastroping_doorsLocked", true, true];

2
