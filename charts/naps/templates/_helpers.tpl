{{- define "naps.fullname" -}}
{{ .Release.Name }}-naps
{{- end }}

{{- define "naps.labels" -}}
app.kubernetes.io/name: naps
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}
