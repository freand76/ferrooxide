// Constructs the parser accepts before elaboration supports them (parse-only test).
package pkg;
  typedef enum { RED, GREEN } color_e;
  import "DPI-C" context function int c_add(input int a, input int b);
  export "DPI-C" function sv_cb;
  function void sv_cb(); endfunction
endpackage

interface class Shape;
  pure virtual function real area();
endclass

virtual class Base #(type T = int, int N = 4);
  typedef T elem_t;
  protected rand T data[N];
  static int count = 0;
  local bit [3:0] flags;
  constraint c_data { foreach (data[i]) data[i] inside {[0:10]}; soft data[0] == 1; }
  extern virtual function void show();
  pure virtual task run(input int n);
  function new(int seed = 0); count++; endfunction
endclass

class Derived extends Base #(byte, 2) implements Shape;
  rand int unsigned len;
  randc bit [1:0] kind;
  elem_t first;
  mailbox #(int) mb;
  semaphore sem;
  covergroup cg @(posedge top.clk);
    option.per_instance = 1;
    cp_len: coverpoint len { bins small[] = {[0:3]}; bins big = {[4:$]}; illegal_bins bad = default; }
    cp_kind: coverpoint kind iff (len > 0) { wildcard bins odd = {2'b?1}; bins t = (0 => 1 => 2), (1 => 3[*2]); }
    cross cp_len, cp_kind { ignore_bins ig = binsof(cp_len.small) && binsof(cp_kind) intersect {0}; }
  endgroup
  constraint c_len { len dist { 0 := 1, [1:5] :/ 4 }; len < 10 -> kind != 0; solve len before kind; }
  function new(); super.new(1); cg = new; endfunction
  virtual function real area(); return 1.0; endfunction
  virtual task run(input int n);
    fork
      begin : a #1; end
      begin wait (len > 0); end
    join_any
    disable fork;
    wait fork;
  endtask
endclass

function void Base::show(); $display("%p", data); endfunction

module top;
  import pkg::*;
  logic clk = 0, req, ack;
  int q[$], dyn[], aa[string];
  logic [7:0] bytes [4];
  Derived d;
  default clocking cb @(posedge clk);
    default input #1step output #0;
    input ack;
    output negedge req;
  endclocking
  property p_handshake(x, y);
    @(posedge clk) disable iff (!req) x |-> ##[1:3] y ##1 !y[*2] or (x and y) |=> s_eventually y;
  endproperty
  sequence s_req; req ##1 ack[->1] ##1 (ack, q.push_back(1)); endsequence
  a_hs: assert property (p_handshake(req, ack)) else $error("handshake");
  c_req: cover property (@(posedge clk) s_req);
  assume property (@(posedge clk) req |-> not ack until_with req);
  let max(a, b) = a > b ? a : b;
  always #5 clk = ~clk;
  initial begin
    automatic int k = 0;
    d = new();
    void'(d.randomize() with { len == 3; });
    if (!std::randomize(k) with { k inside {[1:9]}; }) $fatal(1, "randomize");
    q = {1, 2, 3};
    q.push_front(0);
    dyn = new[4](dyn);
    aa["x"] = 1;
    foreach (aa[s]) $display(s);
    k = q.sum() with (item * 2);
    bytes = {<< byte {32'hdeadbeef}};
    {>> {k}} = bytes;
    randcase 1: k = 1; 2: k = 2; endcase
    randsequence (main)
      main : first second := 2 | second;
      first : { k++; };
      second : repeat (2) third;
      third : if (k > 1) first else second;
    endsequence
    assert (k > 0) else $warning("k");
    assert #0 (k != 5);
    cb.req <= ##2 1'b1;
    ##1;
    ->> #2 ev;
    case (k) inside [0:3]: ; default: ; endcase
    k = max(k, 3);
    $display("%s", RED.name());
  end
  event ev;
endmodule

checker c_onehot(logic [3:0] v, event clk);
  a1: assert property (@clk $onehot(v));
endchecker

program automatic prog(input logic clk);
  initial begin @(posedge clk); $finish; end
endprogram

bind top c_onehot u_chk(.v(top.bytes[0][3:0]), .clk(top.clk));

config cfg;
  design work.top;
  default liblist work;
endconfig
