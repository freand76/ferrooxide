// More class values: conditionals of handles (IEEE 1800-2017 11.4.11), class handle
// members of unpacked structs (null at first), enum methods of properties reached through
// array elements, methods returning queues, $right & co. of queue properties (20.7),
// assignments to parts of vector properties, methods of static properties reached through
// a handle, and queue methods with `with` over a property of structs (7.12).
typedef enum {LOCK, GRANT} req_t;
class W;
  int k;
endclass
typedef struct {W m_type; string m_name;} pair_t;
typedef struct {string name; int v;} al_t;
class R;
  req_t request;
endclass
class Q;
  int n;
  function void inc();
    n++;
  endfunction
endclass
class O;
  typedef W wq_t[$];
  pair_t ovrd;
  R rq[$];
  W ws[$];
  int q[$];
  bit [127:0] m_bits;
  int it;
  al_t als[$];
  static Q tw = new;
  static O inst = new;
  function O pick(O o);
    return (o == null) ? this : o;
  endfunction
  function string first_name();
    return rq[0].request.name();
  endfunction
  virtual function wq_t get_all();
    return ws;
  endfunction
  function int last();
    return $right(q);
  endfunction
  function void pack(bit [63:0] v, int size);
    for (int i = 0; i < size; i++) m_bits[it + i] = v[i];
    it += size;
  endfunction
  static function int bump();
    inst.tw.inc();
    inst.tw.inc();
    return inst.tw.n;
  endfunction
  function int count(string n);
    al_t r[$];
    r = als.find(i) with (i.name == n && i.v > 0);
    return r.size();
  endfunction
endclass

module t;
  integer errors = 0;
  task check(input integer line, input [127:0] got, input [127:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask

  O a, b;
  W w, x;
  R r;
  W all[$];
  al_t e;
  pair_t p;

  initial begin
    a = new;
    b = new;
    check(`__LINE__, a.pick(null) == a, 1);
    check(`__LINE__, a.pick(b) == b, 1);
    check(`__LINE__, a.ovrd.m_type == null, 1);
    check(`__LINE__, p.m_type == null, 1);
    w = new;
    w.k = 4;
    a.ovrd.m_type = w;
    x = a.ovrd.m_type;
    check(`__LINE__, x.k, 4);
    p.m_type = w;
    check(`__LINE__, p.m_type.k, 4);
    r = new;
    r.request = GRANT;
    a.rq.push_back(r);
    if (a.first_name() != "GRANT" || a.rq[0].request.name() != "GRANT") begin
      $display("FAILED line %0d", `__LINE__);
      errors = errors + 1;
    end
    a.ws = '{w, x};
    all = a.get_all();
    check(`__LINE__, all.size(), 2);
    a.q = '{5, 6, 7};
    check(`__LINE__, a.last(), 2);
    check(`__LINE__, $size(a.q), 3);
    a.pack(64'hA5, 8);
    a.pack(64'h3, 4);
    a.m_bits[127:120] = 8'hEE;
    a.m_bits[100 +: 4] = 4'h9;
    check(`__LINE__, a.m_bits, 128'hee0000900000000000000000000003a5);
    check(`__LINE__, O::bump(), 2);
    e.name = "x"; e.v = 1; a.als.push_back(e);
    e.name = "y"; e.v = 2; a.als.push_back(e);
    e.name = "x"; e.v = 0; a.als.push_back(e);
    check(`__LINE__, a.count("x"), 1);
    check(`__LINE__, a.count("y"), 1);
    if (errors == 0) $display("PASSED");
  end
endmodule
