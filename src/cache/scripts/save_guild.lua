local guild_key = KEYS[1]
local guild = ARGS[1]

redis.call("JSON.SET", guild_key, ".", guild)

for i = 2, table.maxn(ARGS) do
	redis.call("JSON.SET", KEYS[i], ".", ARGS[i])
end
