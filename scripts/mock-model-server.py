#!/usr/bin/env python3
"""Offline model endpoints used only by the isolated integration smoke test."""
import argparse,json,pathlib,time
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(json.dumps({'data':[{'id':'mock-model','object':'model'}]}).encode())
 def do_POST(self):
  body=self.rfile.read(int(self.headers.get('Content-Length','0')));data=json.loads(body)
  log=pathlib.Path(self.server.log_dir);log.mkdir(parents=True,exist_ok=True)
  name='claude' if 'messages' in self.path else 'codex';(log/(name+'-request.json')).write_bytes(body)
  if name=='claude':
   msg={'id':'msg_bridge_mock','type':'message','role':'assistant','model':data.get('model','mock-model'),'content':[{'type':'text','text':'BRIDGE_ORCHID remembered from the imported conversation.'}],'stop_reason':'end_turn','stop_sequence':None,'usage':{'input_tokens':20,'output_tokens':10}}
   if not data.get('stream'):
    self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(json.dumps(msg).encode());return
   events=[('message_start',{'type':'message_start','message':{**msg,'content':[],'stop_reason':None}}),('content_block_start',{'type':'content_block_start','index':0,'content_block':{'type':'text','text':''}}),('content_block_delta',{'type':'content_block_delta','index':0,'delta':{'type':'text_delta','text':msg['content'][0]['text']}}),('content_block_stop',{'type':'content_block_stop','index':0}),('message_delta',{'type':'message_delta','delta':{'stop_reason':'end_turn','stop_sequence':None},'usage':{'output_tokens':10}}),('message_stop',{'type':'message_stop'})]
  else:
   message={'id':'msg_bridge_mock','type':'message','role':'assistant','status':'completed','content':[{'type':'output_text','text':'BRIDGE_ORCHID remembered from the imported conversation.','annotations':[]}]}
   response={'id':'resp_bridge_mock','object':'response','created_at':int(time.time()),'status':'completed','model':data.get('model','mock-model'),'output':[message],'usage':{'input_tokens':20,'output_tokens':10,'total_tokens':30}}
   events=[('response.created',{'type':'response.created','response':{**response,'status':'in_progress','output':[]}}),('response.output_item.added',{'type':'response.output_item.added','output_index':0,'item':{**message,'status':'in_progress','content':[]}}),('response.content_part.added',{'type':'response.content_part.added','item_id':message['id'],'output_index':0,'content_index':0,'part':{'type':'output_text','text':'','annotations':[]}}),('response.output_text.delta',{'type':'response.output_text.delta','item_id':message['id'],'output_index':0,'content_index':0,'delta':message['content'][0]['text']}),('response.output_item.done',{'type':'response.output_item.done','output_index':0,'item':message}),('response.completed',{'type':'response.completed','response':response})]
  self.send_response(200);self.send_header('Content-Type','text/event-stream');self.end_headers()
  for name,event in events:
   try:self.wfile.write(('event: '+name+'\ndata: '+json.dumps(event)+'\n\n').encode());self.wfile.flush()
   except BrokenPipeError:break
 def log_message(self,*args):pass
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=18439);p.add_argument('--log-dir',default='/tmp/asb-integration');a=p.parse_args();s=ThreadingHTTPServer(('127.0.0.1',a.port),Handler);s.log_dir=a.log_dir;s.serve_forever()
