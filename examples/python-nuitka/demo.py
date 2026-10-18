"""Preview a local update safely. The installer callback consumes a harmless fixture."""
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import argparse
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'bindings/python'))
from winsparkle import WinSparkle

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--dll', type=Path, default=ROOT/'target/x86_64-pc-windows-msvc/debug/WinSparkle.dll')
    parser.add_argument('--lang', default='en')
    parser.add_argument('--theme', choices=('auto', 'light', 'dark'), default='auto')
    parser.add_argument('--one-click', action=argparse.BooleanOptionalAction, default=True,
                        help='Download and install after one confirmation (default); --no-one-click uses two steps')
    parser.add_argument('--seconds', type=int, default=300)
    args = parser.parse_args()
    payload = b'Harmless update preview fixture\n'*65536
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def do_GET(self):
            feed = f'''<rss xmlns:s="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><item>
            <s:version>2.1.0</s:version><s:shortVersionString>2.1</s:shortVersionString>
            <description><![CDATA[<h2>Lumen Notes 2.1</h2><p>A simpler way to capture ideas and organize your notes.</p>
            <ul><li>Faster notebook search</li><li>Improved Markdown preview</li><li>Interface fixes and new keyboard shortcuts</li></ul>]]></description>
            <enclosure url="http://127.0.0.1:{server.server_port}/update.exe" length="{len(payload)}"/>
            </item></channel></rss>'''.encode()
            data = feed if self.path == '/feed' else payload
            self.send_response(200); self.send_header('Content-Length', str(len(data))); self.end_headers()
            try:
                for i in range(0,len(data),8192):
                    self.wfile.write(data[i:i+8192])
                    if self.path != '/feed': time.sleep(.03)
            except (OSError,ConnectionError): pass
    server = ThreadingHTTPServer(('127.0.0.1',0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    done = threading.Event()
    with WinSparkle(args.dll) as ws:
        ws.configure('WinSparkle-NG preview','Lumen Notes','2.0',f'http://127.0.0.1:{server.server_port}/feed',lang=args.lang,one_click=args.one_click,theme=args.theme)
        ws.use_settings({'CheckForUpdates':'0'})
        ws.on('user_run_installer',lambda path: (print('Preview consumed:',path,flush=True),1)[1])
        ws.on('shutdown_request',done.set)
        ws.on('update_dismissed',done.set)
        ws.check()
        print('Preview ready',flush=True)
        done.wait(args.seconds)
    server.shutdown(); server.server_close(); thread.join()

if __name__ == '__main__': main()
