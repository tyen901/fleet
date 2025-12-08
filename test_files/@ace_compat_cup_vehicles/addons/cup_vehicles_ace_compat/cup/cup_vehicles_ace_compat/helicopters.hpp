class CUP_AH6_BASE;
class CUP_MH6_TRANSPORT: CUP_AH6_BASE
{
	ace_fastroping_enabled = 1;
	ace_fastroping_ropeOrigins[] = {"ropeOriginLeft", "ropeOriginRight"};
};
class CUP_B_MH6M_OBS_USA: CUP_MH6_TRANSPORT
{
	ace_fastroping_enabled = 0;
};
class CUP_B_MH6J_OBS_USA: CUP_MH6_TRANSPORT
{
	ace_fastroping_enabled = 0;
};

class CUP_AW159_Unarmed_Base: Helicopter_Base_H
{
	ace_fastroping_enabled = 2;
	ace_fastroping_friesType = "ACE_friesGantry";
	ace_fastroping_friesAttachmentPoint[] = {1.05, 1, 1.3}; //left/right, forward/backward, up/down
	ace_fastroping_onCut = "ace_fastroping_fnc_onCutCommon";
	ace_fastroping_onPrepare = "ace_fastroping_fnc_onPrepareCommon";
	ace_fastroping_ropeOrigins[] = {"ropeOriginLeft", "ropeOriginRight"};
	EQUIP_FRIES_ATTRIBUTE;

	class UserActions
	{
		class CloseRdoor
		{
			condition = "this doorPhase ""CargoDoorR"" > 0.5 AND ((this getCargoIndex player) isEqualTo 0) && !(this getVariable ['ace_fastroping_doorsLocked', false])";
		};
	};
};

class CUP_CH47F_base: Helicopter_Base_H
{
	ace_fastroping_enabled = 1;
	ace_fastroping_ropeOrigins[] = {"ropeOriginLeft", "ropeOriginRight", "ropeOriginMid"};
};

class CUP_CH53E_base: Helicopter_Base_H
{
	ace_fastroping_enabled = 1;
	ace_fastroping_ropeOrigins[] = {"ropeOriginLeft", "ropeOriginRight"};

	class UserActions
	{
		class RampClose
		{
			condition = "this animationPhase ""ramp_bottom"" >= 0.56 && player == driver this && !(this getVariable ['ace_fastroping_doorsLocked', false])";
		};
	};
};

class CUP_Merlin_HC3_Base: Helicopter_Base_H
{
	ace_fastroping_enabled = 1;
	ace_fastroping_ropeOrigins[] = {"ropeOriginRight"};
	ace_fastroping_onPrepare = "CUP_fnc_fastroping_onPrepareHC3";
	ace_fastroping_onCut = "CUP_fnc_fastroping_onCutHC3";

	class UserActions
	{
		class CloseRdoor
		{
			condition = "this doorPhase 'dvere_p' > 0.5 && {(this getCargoIndex player) isEqualTo 0} && {!(this getVariable ['ace_fastroping_doorsLocked', false])}";
		};
		class OutWinch
		{
			condition = "false";
		};
		class InWinch
		{
			condition = "false";
		};
	};
};

class CUP_Ka60_Base: Helicopter_Base_H
{
	ace_fastroping_enabled = 1;
	ace_fastroping_ropeOrigins[] = {"ropeOriginLeft", "ropeOriginRight"};
};

class CUP_MH60S_Base: Helicopter_Base_H
{
	ace_fastroping_enabled = 1;
	ace_fastroping_ropeOrigins[] = {"ropeOriginRight"};

	class UserActions
	{
		class OpenDoors;
		class CloseDoors: OpenDoors
		{
			condition = "this animationPhase ""doors"" > 0.5 AND driver this == player AND Alive(this) && !(this getVariable ['ace_fastroping_doorsLocked', false])";
		};
	};
};

class CUP_Mi8_base: Helicopter_Base_H
{
	ace_fastroping_enabled = 1;
	ace_fastroping_ropeOrigins[] = {"ropeOriginLeft"};
};

class CUP_Mi8_medevac_base: CUP_Mi8_base
{
	ace_fastroping_enabled = 0;
};

class CUP_Mi171Sh_Base: CUP_Mi8_base
{
	ace_fastroping_enabled = 1;
	ace_fastroping_ropeOrigins[] = {"ropeOriginLeft", "ropeOriginRight"};
};

class CUP_SA330_Base: Helicopter_Base_H
{
	ace_fastroping_enabled = 1;
	ace_fastroping_ropeOrigins[] = {"ropeOriginLeft", "ropeOriginRight"};

	class UserActions
	{
		class CloseRdoor
		{
			condition = "alive this && {this doorPhase 'ofrp_puma_porte_droite' > 0.5} && {(this getCargoIndex player) isEqualTo 0} && {!(this getVariable ['ace_fastroping_doorsLocked', false])}";
		};
		class CloseLdoor
		{
			condition = "alive this && {this doorPhase 'ofrp_puma_porte_gauche' > 0.5} && {(this getCargoIndex player) isEqualTo 1} && !(this getVariable ['ace_fastroping_doorsLocked', false])";
		};
	};
};

class CUP_UH1H_base: Helicopter_Base_H
{
	ace_fastroping_enabled = 2;
	ace_fastroping_friesType = "ACE_friesAnchorBar";
	ace_fastroping_friesAttachmentPoint[] = {0, 1.45, -0.3}; //left/right, forward/backward, up/down
	ace_fastroping_onCut = "ace_fastroping_fnc_onCutCommon";
	ace_fastroping_onPrepare = "ace_fastroping_fnc_onPrepareCommon";
	ace_fastroping_ropeOrigins[] = {"ropeOriginLeft", "ropeOriginRight"};
	EQUIP_FRIES_ATTRIBUTE;
};

class CUP_B_UH1Y_Base;
class CUP_B_UH1Y_UNA_USMC: CUP_B_UH1Y_Base //only version with support for fastrope
{
	ace_fastroping_enabled = 1;
	ace_fastroping_ropeOrigins[] = {"ropeOriginLeft", "ropeOriginRight"};
	ace_fastroping_onPrepare = "CUP_fnc_fastroping_onPrepareUH1Y";
	ace_fastroping_onCut = "CUP_fnc_fastroping_onCutUH1Y";
};
class CUP_B_UH1Y_MEV_USMC: CUP_B_UH1Y_UNA_USMC
{
	ace_fastroping_enabled = 0;
};

class CUP_UH60_Base: Helicopter_Base_H
{
	ace_fastroping_enabled = 1;
	ace_fastroping_ropeOrigins[] = {"ropeOriginRight"};

	class UserActions
	{
		class OpenDoors;
		class CloseDoors: OpenDoors
		{
			condition = "this animationPhase ""doors"" > 0.5 AND driver this == player AND Alive(this) && !(this getVariable ['ace_fastroping_doorsLocked', false])";
		};
	};
};
