/* Probe for spec/vendor/ruby_gvl_spec.rb: calls Ruby's C API directly, so the vendor specs
 * observe Ruby itself, not the outpunch binding. Built into a temporary directory by the spec. */
#include <ruby.h>
#include <ruby/thread.h>
#include <time.h>
#include <unistd.h>

static void *mark_ran(void *flag) { *(int *)flag = 1; return NULL; }
static void ignore_ubf(void *arg) { (void)arg; }

static double now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1e3 + ts.tv_nsec / 1e6;
}

/* With pending = true, Thread#wakeup on the current thread leaves an interrupt pending, with
 * no Ruby code (so no interrupt check) before the call. Returns whether the function ran. */
static VALUE call_after_wakeup(VALUE self, VALUE pending) {
    int ran = 0;
    if (RTEST(pending)) rb_thread_wakeup(rb_thread_current());
    rb_thread_call_without_gvl2(mark_ran, &ran, ignore_ubf, NULL);
    return ran ? Qtrue : Qfalse;
}

/* Holds the GVL for ms milliseconds without checking interrupts, then calls. Returns whether
 * the function ran. */
static VALUE call_after_holding_gvl(VALUE self, VALUE ms) {
    int ran = 0;
    double end = now_ms() + NUM2DBL(ms);
    while (now_ms() < end) {}
    rb_thread_call_without_gvl2(mark_ran, &ran, ignore_ubf, NULL);
    return ran ? Qtrue : Qfalse;
}

static volatile int unblocked;
static int func_returned, after_check;

static void *wait_until_unblocked(void *ms) {
    for (long n = (long)ms; !unblocked && n > 0; n--) usleep(1000);
    func_returned = 1;
    return NULL;
}
static void unblock(void *arg) { (void)arg; unblocked = 1; }

/* Blocks without the GVL for up to ms milliseconds or until the unblocking function is called,
 * then calls rb_thread_check_ints, as the outpunch binding does. */
static VALUE block_then_check_ints(VALUE self, VALUE ms) {
    unblocked = 0; func_returned = 0; after_check = 0;
    rb_thread_call_without_gvl2(wait_until_unblocked, (void *)NUM2LONG(ms), unblock, NULL);
    rb_thread_check_ints();
    after_check = 1;
    return Qnil;
}

/* [unblocking function called, function returned, rb_thread_check_ints returned] */
static VALUE last_block(VALUE self) {
    return rb_ary_new_from_args(3, unblocked ? Qtrue : Qfalse, func_returned ? Qtrue : Qfalse,
                                after_check ? Qtrue : Qfalse);
}

void Init_gvl_probe(void) {
    VALUE m = rb_define_module("GvlProbe");
    rb_define_module_function(m, "call_after_wakeup", call_after_wakeup, 1);
    rb_define_module_function(m, "call_after_holding_gvl", call_after_holding_gvl, 1);
    rb_define_module_function(m, "block_then_check_ints", block_then_check_ints, 1);
    rb_define_module_function(m, "last_block", last_block, 0);
}
