#define EQUIP_FRIES_ATTRIBUTE \
	class Attributes \
	{ \
		class ace_fastroping_equipFRIES \
		{ \
			property = "ace_fastroping_equipFRIES"; \
			control = "Checkbox"; \
			displayName = "$STR_ace_fastroping_Eden_equipFRIES"; \
			tooltip = "$STR_ace_fastroping_Eden_equipFRIES_Tooltip"; \
			expression = "[_this] call ace_fastroping_fnc_equipFRIES"; \
			typeName = "BOOL"; \
			condition = "objectVehicle"; \
			defaultValue = 0; \
		}; \
	}
