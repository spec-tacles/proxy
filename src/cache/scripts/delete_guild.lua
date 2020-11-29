local guild_key = KEYS[1]

redis.call("DEL", guild_key)

for i = 1, table.maxn(ARGS) do
	redis.call("DEL", KEYS[i])
end
