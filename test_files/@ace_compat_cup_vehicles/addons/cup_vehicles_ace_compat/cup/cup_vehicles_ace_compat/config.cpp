////////////////////////////////////////////////////////////////////
//DeRap: config.bin
//Produced from mikero's Dos Tools Dll version 10.06
//https://mikero.bytex.digital/Downloads
//'now' is Tue Dec 09 08:40:15 2025 : 'file' last modified on Fri Jun 11 03:47:47 2021
////////////////////////////////////////////////////////////////////

#define _ARMA_

class CfgPatches
{
	class CUP_Vehicles_ACE_compat
	{
		units[] = {};
		weapons[] = {};
		requiredVersion = 0.1;
		requiredAddons[] = {"CUP_AirVehicles_CH53E","CUP_AirVehicles_HC3","CUP_AirVehciles_KA60","CUP_AirVehciles_SA330","CUP_AirVehicles_UH60","CUP_WheeledVehicles_MTVR","CUP_WheeledVehicles_T810","CUP_WheeledVehicles_Ural","CUP_WheeledVehicles_V3S","ace_interaction"};
		author = "Community Upgrade Project";
		magazines[] = {};
		ammo[] = {};
	};
};
class CfgAmmo{};
class CfgMagazines{};
class CfgWeapons{};
class CfgVehicles
{
	class LandVehicle;
	class Car: LandVehicle
	{
		class ACE_Actions
		{
			class ACE_MainActions{};
		};
	};
	class Car_F: Car{};
	class Helicopter;
	class Helicopter_Base_F: Helicopter{};
	class Helicopter_Base_H: Helicopter_Base_F{};
	class CUP_MTVR_Base: Car_F{};
	class CUP_MTVR_Refuel_Base: CUP_MTVR_Base
	{
		transportFuel = 0;
		ace_refuel_hooks[] = {{-1.09,-0.01,-0.5},{1,-0.01,-0.5}};
		ace_refuel_fuelCargo = 10000;
	};
	class CUP_MTVR_Reammo_Base: CUP_MTVR_Base
	{
		transportAmmo = 0;
		ace_rearm_defaultSupply = 1200;
	};
	class CUP_MTVR_Repair_Base: CUP_MTVR_Base
	{
		transportRepair = 0;
		ace_repair_canRepair = 1;
	};
	class CUP_V3S_Open_Base: Car_F{};
	class CUP_V3S_Refuel_Base: CUP_V3S_Open_Base
	{
		transportFuel = 0;
		ace_refuel_hooks[] = {{-0.35,-3.35,-0.4},{0.4,-3.35,-0.4}};
		ace_refuel_fuelCargo = 6500;
	};
	class CUP_V3S_Repair_Base: CUP_V3S_Open_Base
	{
		transportRepair = 0;
		ace_repair_canRepair = 1;
	};
	class CUP_V3S_Rearm_Base: CUP_V3S_Open_Base
	{
		transportAmmo = 0;
		ace_rearm_defaultSupply = 1200;
	};
	class Truck_F: Car_F{};
	class CUP_Ural_BaseTurret: Truck_F{};
	class CUP_Ural_Support_Base: CUP_Ural_BaseTurret{};
	class CUP_Ural_Refuel_Base: CUP_Ural_Support_Base
	{
		transportFuel = 0;
		ace_refuel_hooks[] = {{-0.05,-3.65,-0.42}};
		ace_refuel_fuelCargo = 10000;
	};
	class CUP_Ural_Reammo_Base: CUP_Ural_Support_Base
	{
		transportAmmo = 0;
		ace_rearm_defaultSupply = 1200;
	};
	class CUP_Ural_Repair_Base: CUP_Ural_Support_Base
	{
		transportRepair = 0;
		ace_repair_canRepair = 1;
	};
	class Truck_02_base_F;
	class Truck_02_fuel_base_F;
	class Truck_02_box_base_F;
	class CUP_Kamaz_5350_Base: Truck_02_base_F{};
	class CUP_Kamaz_5350_Refuel_Base: Truck_02_fuel_base_F
	{
		transportFuel = 0;
		ace_refuel_hooks[] = {{-0.02,-3.33,-1.05}};
		ace_refuel_fuelCargo = 10000;
	};
	class CUP_Kamaz_5350_ReAmmo_Base: CUP_Kamaz_5350_Base
	{
		transportAmmo = 0;
		ace_rearm_defaultSupply = 1200;
	};
	class CUP_Kamaz_5350_Repair_Base: Truck_02_box_base_F
	{
		transportRepair = 0;
		ace_repair_canRepair = 1;
	};
	class CUP_T810_Base: Car_F{};
	class CUP_T810_Unarmed_Base: CUP_T810_Base{};
	class CUP_T810_Refuel_Base: CUP_T810_Unarmed_Base
	{
		transportFuel = 0;
		ace_refuel_hooks[] = {{-1.01,0.21,-0.5},{1.08,0.2,-0.5}};
		ace_refuel_fuelCargo = 10000;
	};
	class CUP_T810_Reammo_Base: CUP_T810_Unarmed_Base
	{
		transportAmmo = 0;
		ace_rearm_defaultSupply = 1200;
	};
	class CUP_T810_Repair_Base: CUP_T810_Unarmed_Base
	{
		transportRepair = 0;
		ace_repair_canRepair = 1;
	};
	class CUP_AW159_Unarmed_Base: Helicopter_Base_H
	{
		ace_fastroping_enabled = 2;
		ace_fastroping_friesType = "ACE_friesGantry";
		ace_fastroping_friesAttachmentPoint[] = {1.05,1,1.3};
		ace_fastroping_onCut = "ace_fastroping_fnc_onCutCommon";
		ace_fastroping_onPrepare = "ace_fastroping_fnc_onPrepareCommon";
		ace_fastroping_ropeOrigins[] = {"ropeOriginLeft","ropeOriginRight"};
		class Attributes
		{
			class ace_fastroping_equipFRIES
			{
				property = "ace_fastroping_equipFRIES";
				control = "Checkbox";
				displayName = "$STR_ace_fastroping_Eden_equipFRIES";
				tooltip = "$STR_ace_fastroping_Eden_equipFRIES_Tooltip";
				expression = "[_this] call ace_fastroping_fnc_equipFRIES";
				typeName = "BOOL";
				condition = "objectVehicle";
				defaultValue = 0;
			};
		};
		class UserActions
		{
			class CloseRdoor
			{
				condition = "this doorPhase ""CargoDoorR"" > 0.5 AND ((this getCargoIndex player) isEqualTo 0) && !(this getVariable ['ace_fastroping_doorsLocked', false])";
			};
		};
	};
	class CUP_CH53E_base: Helicopter_Base_H
	{
		ace_fastroping_enabled = 1;
		ace_fastroping_ropeOrigins[] = {"ropeOriginLeft","ropeOriginRight"};
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
		ace_fastroping_ropeOrigins[] = {"ropeOriginLeft","ropeOriginRight"};
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
	class CUP_SA330_Base: Helicopter_Base_H
	{
		ace_fastroping_enabled = 1;
		ace_fastroping_ropeOrigins[] = {"ropeOriginLeft","ropeOriginRight"};
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
};
class CfgFunctions
{
	class CUP
	{
		class Vehicles_ACE_compat
		{
			class fastroping_onCutHC3
			{
				file = "\CUP\CUP_Vehicles_ACE_compat\functions\fnc_fastroping_onCutHC3.sqf";
				recompile = 0;
			};
			class fastroping_onPrepareHC3
			{
				file = "\CUP\CUP_Vehicles_ACE_compat\functions\fnc_fastroping_onPrepareHC3.sqf";
				recompile = 0;
			};
			class fastroping_onCutUH1Y
			{
				file = "\CUP\CUP_Vehicles_ACE_compat\functions\fnc_fastroping_onCutUH1Y.sqf";
				recompile = 0;
			};
			class fastroping_onPrepareUH1Y
			{
				file = "\CUP\CUP_Vehicles_ACE_compat\functions\fnc_fastroping_onPrepareUH1Y.sqf";
				recompile = 0;
			};
		};
	};
};
